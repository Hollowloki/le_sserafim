use arrayvec::ArrayVec;
use hashbrown::hash_map::Entry;
use hashbrown::HashMap;
use rand::Rng;
use termcolor::StandardStream;

use crate::allocator::GLOBAL;
use crate::cc_parser::ast::Type;
use crate::vm::value::Value;
use crate::{
    allocator::allocation::{CeAlloc, CeAllocation},
    vm::object::StringObject,
};
use rustc_hash::FxHasher;
use std::hash::BuildHasherDefault;
use std::{mem, ptr};

use self::built_in::ArrayMethod;
use self::compiler::Compiler;
use self::error::{AttributeError, Error, ErrorS, IndexError, OverflowError, Result, TypeError};
use self::object::{
    ArrayObject, BoundArrayMethodObject, BoundMethodObject, ClosureObject, InstanceObject, Native,
    ObjectFunction, ObjectNative, ObjectType, StructObject, UpvalueObject,
};
pub mod built_in;
pub mod chunk;
pub mod compiler;
pub mod error;
pub mod object;
pub mod op;
pub mod value;

const FRAMES_MAX: usize = 64;
const STACK_MAX: usize = FRAMES_MAX * STACK_MAX_PER_FRAME;
const STACK_MAX_PER_FRAME: usize = u8::MAX as usize + 1;

pub struct VM<'a> {
    stack_top: *mut Value,
    stack: Box<[Value; STACK_MAX]>,

    frames: ArrayVec<CallFrame, FRAMES_MAX>,
    frame: CallFrame,

    allocator: &'a mut CeAllocation,
    next_gc: usize,

    globals: HashMap<*mut StringObject, Value, BuildHasherDefault<FxHasher>>,
    open_upvalues: Vec<*mut UpvalueObject>,

    struct_init_method: *mut StringObject,
}

impl<'a> VM<'a> {
    pub fn new(allocator: &'a mut CeAllocation) -> VM {
        let mut globals = HashMap::with_capacity_and_hasher(256, BuildHasherDefault::default());

        let clock_string = allocator.alloc("clock");
        let random_number = allocator.alloc("random_number");

        let clock_native = allocator.alloc(ObjectNative::new(Native::Clock));
        let random_number_native = allocator.alloc(ObjectNative::new(Native::RandomNumber));

        let struct_init_method = allocator.alloc("new");

        globals.insert(clock_string, clock_native.into());
        globals.insert(random_number, random_number_native.into());
        VM {
            stack: Box::new([Value::default(); STACK_MAX]),
            stack_top: ptr::null_mut(),
            allocator,
            globals,
            frames: ArrayVec::new(),
            frame: CallFrame {
                closure: ptr::null_mut(),
                ip: ptr::null_mut(),
                stack: ptr::null_mut(),
            },
            open_upvalues: Vec::new(),
            next_gc: 1024 * 1024,
            struct_init_method,
        }
    }

    pub fn run(&mut self, source: &str, stdout: &mut StandardStream) -> Result<(), Vec<ErrorS>> {
        let mut compiler = Compiler::new(self.allocator);
        let function = compiler.compile(source, self.allocator, stdout)?;
        self.run_function(function).map_err(|e| vec![e])?;
        Ok(())
    }

    pub fn run_function(&mut self, function: *mut ObjectFunction) -> Result<()> {
        self.stack_top = self.stack.as_mut_ptr();

        self.frames.clear();
        self.frame = CallFrame {
            closure: self
                .allocator
                .alloc(ClosureObject::new(function, Vec::new())),
            ip: unsafe { (*function).chunk.code.as_ptr() },
            stack: self.stack_top,
        };

        loop {
            let function = unsafe { &mut *(*self.frame.closure).function };
            let idx = unsafe { self.frame.ip.offset_from((*function).chunk.code.as_ptr()) };
            (*function).chunk.disassemble_instruction(idx as usize);

            match self.read_u8() {
                op::ARRAY_ACCESS => self.op_array_access(),
                op::ARRAY_ACCESS_ASSIGN => self.op_array_access_assign(),
                op::ARRAY => self.op_array(),
                op::GET_ARRAY => self.op_get_array(),
                op::GET_SUPER => self.op_get_super(),
                op::INHERIT => self.op_inherit(),
                op::SUPER_INVOKE => self.op_super_invoke(),
                op::INVOKE => self.op_invoke(),
                op::SET_FIELD => self.op_set_field(),
                op::GET_FIELD => self.op_get_field(),
                op::FIELD => self.op_field(),
                op::STRUCT => self.op_cstruct(),
                op::METHOD => self.op_method(),
                op::CECILE_CONSTANT => self.c_constant(),
                op::ADD => self.op_binary_add(),
                op::CONCAT => self.op_concat(),
                op::SUB => self.sub(),
                op::MUL => self.mul(),
                op::DIV => self.div(),
                op::EQUAL => self.equal(),
                op::NOT_EQUAL => self.not_equal(),
                op::NEG => self.negate(),
                op::MODULO => self.modulo(),
                op::GREATER_THAN => self.greater(),
                op::GREATER_THAN_EQUAL => self.greater_equal(),
                op::PRINT => self.op_print(),
                op::PRINT_LN => self.op_print_ln(),
                op::CALL => self.call(),
                op::CLOSURE => self.closure(),
                op::LOOP => self.loop_(),
                op::JUMP => self.jump(),
                op::JUMP_IF_FALSE => self.jump_if_false(),
                op::GET_LOCAL => self.get_local(),
                op::SET_LOCAL => self.set_local(),
                op::SET_UPVALUE => self.set_upvalue(),
                op::GET_UPVALUE => self.get_upvalue(),
                op::SET_GLOBAL => self.set_global(),
                op::LESS_THAN => self.less(),
                op::LESS_THAN_EQUAL => self.less_equal(),
                op::GET_GLOBAL => self.get_global(),
                op::DEFINE_GLOBAL => self.define_global(),
                op::TRUE => self.op_true(),
                op::FALSE => self.op_false(),
                op::NIL => self.op_nil(),
                op::CLOSE_UPVALUE => self.close_upvalue(),
                op::POP => self.op_pop(),

                op::RETURN => {
                    let value = self.pop();

                    self.stack_top = self.frame.stack;
                    match self.frames.pop() {
                        Some(frame) => self.frame = frame,
                        None => break,
                    }
                    self.push_to_stack(value);
                    Ok(())
                }
                _ => todo!(),
            }?;

            // print top of stack element
            print!("    ");
            let mut stack_ptr = self.frame.stack;
            while stack_ptr < self.stack_top {
                print!("[ {} ]", unsafe { *stack_ptr });
                stack_ptr = unsafe { stack_ptr.add(1) };
            }
            println!();
        }
        Ok(())
    }

    fn op_array_access_assign(&mut self) -> Result<()> {
        let value = self.pop();
        let index = self.pop();
        let array = self.pop();
        let len = match array.as_object().type_() {
            ObjectType::Array(type_) => {
                let array = unsafe { array.as_object().array };
                unsafe { (*array).values.len() }
            }
            _ => {
                return self.err(TypeError::NotIndexable {
                    type_: array.type_().to_string(),
                })
            }
        };

        if index.is_number() && index.as_number() as usize >= len {
            return self.err(IndexError::IndexOutOfRange {
                index: index.as_number() as usize,
                len: (len - 1),
            });
        }

        match array.as_object().type_() {
            ObjectType::Array(type_) => {
                let array = unsafe { array.as_object().array };
                if index.is_number() {
                    let index = index.as_number() as usize;
                    let arr_value = unsafe { (*array).values.get_unchecked_mut(index) };
                    *arr_value = value;
                } else {
                    return self.err(TypeError::NotIndexable {
                        type_: unsafe { (*array).main.type_.to_string().clone() },
                    });
                }
            }
            _ => {
                return self.err(TypeError::NotIndexable {
                    type_: array.type_().to_string(),
                })
            }
        };
        self.push_to_stack(value);
        Ok(())
    }
    fn op_array_access(&mut self) -> Result<()> {
        let index = self.pop();
        let array = self.pop();
        let len = match array.as_object().type_() {
            ObjectType::Array(type_) => {
                let array = unsafe { array.as_object().array };
                unsafe { (*array).values.len() }
            }
            // ObjectType::String => {
            //     let string = unsafe { array.as_object().string };
            //     unsafe { (*string).value.len() }
            // }
            _ => {
                return self.err(TypeError::NotIndexable {
                    type_: array.type_().to_string(),
                })
            }
        };

        if index.is_number() && index.as_number() as usize >= len {
            return self.err(IndexError::IndexOutOfRange {
                index: index.as_number() as usize,
                len: (len - 1),
            });
        }

        let value = match array.as_object().type_() {
            ObjectType::Array(type_) => {
                let array = unsafe { array.as_object().array };
                if index.is_number() {
                    let index = index.as_number() as usize;
                    let value = unsafe { (*array).values.get_unchecked(index) };
                    *value
                } else {
                    return self.err(TypeError::NotIndexable {
                        type_: unsafe { (*array).main.type_.to_string().clone() },
                    });
                }
            }
            _ => {
                return self.err(TypeError::NotIndexable {
                    type_: array.type_().to_string(),
                })
            }
        };
        self.push_to_stack(value);
        Ok(())
    }

    fn op_array(&mut self) -> Result<()> {
        let arg_count = self.read_u8() as usize;
        let mut array = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            array.push(self.pop());
        }
        let val = array.get(0).unwrap();
        let mut array_type = None;
        if val.is_object() {
            let object = val.as_object();
            match object.type_() {
                ObjectType::Array(t) => {
                    array_type = Some(t);
                }
                _ => {}
            }
        } else if val.is_number() {
            array_type = Some(Type::Int)
        }
        println!("array_type {:?}", array_type);
        array.reverse();
        let array = self.alloc(ArrayObject::new(array, array_type.unwrap()));
        self.push_to_stack(array.into());
        Ok(())
    }

    fn op_get_super(&mut self) -> Result<()> {
        let name = unsafe { self.read_constant().as_object().string };
        let super_ = unsafe { self.pop().as_object().cstruct };
        match unsafe { (*super_).methods.get(&name) } {
            Some(&method) => {
                let instance = unsafe { (*self.peek(0)).as_object().instance };
                let bound_method = self.alloc(BoundMethodObject::new(instance, method));
                self.pop();
                self.push_to_stack(bound_method.into());
            }
            None => {
                return self.err(AttributeError::NoSuchAttribute {
                    type_: unsafe { (*(*super_).name).value.to_string() },
                    name: unsafe { (*name).value.to_string() },
                });
            }
        }
        Ok(())
    }

    fn op_inherit(&mut self) -> Result<()> {
        let cstruct = unsafe { self.pop().as_object().cstruct };
        let super_ = {
            let value = unsafe { *self.peek(0) };
            let object = value.as_object();

            if value.is_object() && object.type_() == ObjectType::Struct {
                unsafe { object.cstruct }
            } else {
                return self.err(TypeError::NotCallable {
                    type_: value.type_().to_string(),
                });
            }
        };

        unsafe { (*cstruct).methods = (*super_).methods.clone() };
        Ok(())
    }

    fn op_super_invoke(&mut self) -> Result<()> {
        let name = unsafe { self.read_constant().as_object().string };
        let arg_count = self.read_u8() as usize;
        let super_ = unsafe { self.pop().as_object().cstruct };
        let instance = unsafe { (*self.peek(arg_count)).as_object().instance };

        match unsafe { (*super_).methods.get(&name) } {
            Some(&method) => self.call_closure(method, arg_count),
            None => self.err(AttributeError::NoSuchAttribute {
                type_: unsafe { (*(*super_).name).value.to_string() },
                name: unsafe { (*name).value.to_string() },
            }),
        };
        Ok(())
    }

    fn op_invoke(&mut self) -> Result<()> {
        let name = unsafe { self.read_constant().as_object().string };
        let arg_count = self.read_u8() as usize;
        let instance = unsafe { (*self.peek(arg_count)).as_object().instance };

        match unsafe { (*instance).fields.get(&name) } {
            Some(&value) => self.call_value(value, arg_count),
            None => match unsafe { (*(*instance).struct_).methods.get(&name) } {
                Some(&method) => self.call_closure(method, arg_count),
                None => self.err(AttributeError::NoSuchAttribute {
                    type_: unsafe { (*(*(*instance).struct_).name).value.to_string() },
                    name: unsafe { (*name).value.to_string() },
                }),
            },
        };
        Ok(())
    }
    fn op_set_field(&mut self) -> Result<()> {
        let name = unsafe { self.read_constant().as_object().string };
        let instance = {
            let value = self.pop();
            let object = value.as_object();

            if value.is_object() && object.type_() == ObjectType::Instance {
                unsafe { object.instance }
            } else {
                return self.err(AttributeError::NoSuchAttribute {
                    type_: value.type_().to_string(),
                    name: unsafe { (*name).value.to_string() },
                });
            }
        };
        let value = self.peek(0);
        unsafe { (*(*instance).struct_).fields.insert(name, *value) };
        Ok(())
    }

    fn op_get_array(&mut self) -> Result<()> {
        let name = unsafe { self.read_constant().as_object().string };
        let array = {
            let value = unsafe { *self.peek(0) };
            let object = value.as_object();

            if value.is_object() {
                match object.type_() {
                    ObjectType::Array(t) => unsafe { object.array },
                    _ => {
                        return self.err(AttributeError::NoSuchAttribute {
                            type_: value.type_().to_string(),
                            name: unsafe { (*name).value.to_string() },
                        });
                    }
                }
            } else {
                return self.err(AttributeError::NoSuchAttribute {
                    type_: value.type_().to_string(),
                    name: unsafe { (*name).value.to_string() },
                });
            }
        };
        let method_type = unsafe { (*array).get_method(name) };
        if method_type.is_none() {
            return self.err(AttributeError::NoSuchAttribute {
                type_: unsafe { ((*array).main.type_).to_string() },
                name: unsafe { (*name).value.to_string() },
            });
        }
        let method_type = method_type.unwrap();

        let bound_arr_method = self.alloc(BoundArrayMethodObject::new(array, method_type));
        self.pop();
        self.push_to_stack(bound_arr_method.into());

        Ok(())
    }

    fn op_get_field(&mut self) -> Result<()> {
        let name = unsafe { self.read_constant().as_object().string };
        let instance = {
            let value = unsafe { *self.peek(0) };
            let object = value.as_object();

            println!("ObjectType {:?}", object.type_());
            if value.is_object() && object.type_() == ObjectType::Instance {
                unsafe { object.instance }
            } else {
                return self.err(AttributeError::NoSuchAttribute {
                    type_: value.type_().to_string(),
                    name: unsafe { (*name).value.to_string() },
                });
            }
        };

        let value = unsafe { (*(*instance).struct_).fields.get(&name) };
        match value {
            Some(&value) => {
                self.pop();
                self.push_to_stack(value);
            }
            None => {
                let method = unsafe { (*(*instance).struct_).methods.get(&name) };
                match method {
                    Some(&method) => {
                        let bound_method = self.alloc(BoundMethodObject::new(instance, method));
                        self.pop();
                        self.push_to_stack(bound_method.into());
                    }
                    None => {
                        return self.err(AttributeError::NoSuchAttribute {
                            type_: "instance".to_string(),
                            name: unsafe { (*name).value.to_string() },
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn op_field(&mut self) -> Result<()> {
        let name = unsafe { self.read_constant().as_object().string };
        let cstruct = unsafe { (*self.peek(0)).as_object().cstruct };
        unsafe { (*cstruct).fields.insert(name, Value::NIL) };
        Ok(())
    }

    fn op_cstruct(&mut self) -> Result<()> {
        let name = unsafe { self.read_constant().as_object().string };
        let cstruct = self.alloc(StructObject::new(name));
        self.push_to_stack(cstruct.into());
        Ok(())
    }

    fn op_method(&mut self) -> Result<()> {
        let name = unsafe { self.read_constant().as_object().string };
        let method = unsafe { self.pop().as_object().closure };
        let cstruct = unsafe { (*self.peek(0)).as_object().cstruct };
        unsafe { (*cstruct).methods.insert(name, method) };
        Ok(())
    }

    fn op_print(&mut self) -> Result<()> {
        let value: value::Value = self.pop();
        print!("{}", value);
        Ok(())
    }

    fn op_print_ln(&mut self) -> Result<()> {
        let value: value::Value = self.pop();
        println!("{}", value);
        Ok(())
    }

    fn op_pop(&mut self) -> Result<()> {
        self.pop();
        Ok(())
    }

    fn c_constant(&mut self) -> Result<()> {
        let constant = self.read_constant();
        self.push_to_stack(constant);
        Ok(())
    }

    fn close_upvalue(&mut self) -> Result<()> {
        self.pop();

        Ok(())
    }

    fn set_upvalue(&mut self) -> Result<()> {
        let upvalue_idx = self.read_u8();
        let upvalue = unsafe {
            *(*self.frame.closure)
                .upvalues
                .get_unchecked(upvalue_idx as usize)
        };
        let value = self.peek(0);
        unsafe { (*upvalue).value = *value };
        Ok(())
    }

    fn get_current_closure_name(&self) -> String {
        unsafe { (*(*(*self.frame.closure).function).name).value.to_string() }
    }

    fn get_current_closure_upvalues(&self) -> Vec<*mut UpvalueObject> {
        unsafe { (*self.frame.closure).upvalues.clone() }
    }

    fn get_upvalue(&mut self) -> Result<()> {
        let upvalue_idx = self.read_u8() as usize;
        let object = *unsafe { (*self.frame.closure).upvalues.get_unchecked(upvalue_idx) };
        let value = unsafe { (*object).value };
        self.push_to_stack(value);
        Ok(())
    }

    fn closure(&mut self) -> Result<()> {
        let function = unsafe { self.read_constant().as_object().function };

        let upvalue_count = unsafe { (*function).upvalue_count } as usize;
        let mut upvalues = Vec::with_capacity(upvalue_count);

        for _ in 0..upvalue_count {
            let is_local = self.read_u8();
            let upvalue_idx = self.read_u8() as usize;

            let upvalue = if is_local != 0 {
                let location = unsafe { *self.frame.stack.add(upvalue_idx) };
                self.capture_upvalue(location)
            } else {
                unsafe { *(*self.frame.closure).upvalues.get_unchecked(upvalue_idx) }
            };
            upvalues.push(upvalue);
        }

        let closure = self.alloc(ClosureObject::new(function, upvalues));
        self.push_to_stack(closure.into());
        Ok(())
    }

    fn call(&mut self) -> Result<()> {
        let arg_count = self.read_u8() as usize;
        let callee = self.peek(arg_count);
        self.call_value(unsafe { *callee }, arg_count as usize)?;
        Ok(())
    }

    fn call_value(&mut self, callee: Value, arg_count: usize) -> Result<()> {
        if callee.is_object() {
            let object = callee.as_object();
            match object.type_() {
                ObjectType::Closure => self.call_closure(unsafe { object.closure }, arg_count),
                ObjectType::Struct => self.call_struct(unsafe { object.cstruct }, arg_count),
                ObjectType::BoundMethod => {
                    self.call_bound_method(unsafe { object.bound_method }, arg_count)
                }
                ObjectType::BoundArrayMethod => {
                    self.call_bound_arr_method(unsafe { object.bound_array_method }, arg_count)
                }
                // ObjectType::Array => self.call_array_method(unsafe { object.array }, arg_count),
                ObjectType::Native => self.call_native(unsafe { object.native }, arg_count),
                _ => self.err(TypeError::NotCallable {
                    type_: callee.type_().to_string(),
                }),
            }
        } else {
            self.err(TypeError::NotCallable {
                type_: callee.type_().to_string(),
            })
        }
    }

    fn call_bound_arr_method(
        &mut self,
        bound_arr_method: *mut BoundArrayMethodObject,
        arg_count: usize,
    ) -> Result<()> {
        let method = unsafe { (*bound_arr_method).method };
        let array = unsafe { (*bound_arr_method).array };
        match method {
            ArrayMethod::Push => {
                if arg_count != 1 {
                    return self.err(TypeError::ArityMisMatch {
                        name: "push".to_string(),
                        expected: 1,
                        actual: arg_count,
                    });
                }
                let value = self.pop();
                if unsafe { (*array).value_type.clone() } != value.type_() {
                    return self.err(TypeError::ArrayValueTypeMismatch {
                        expected: unsafe { (*array).value_type.to_string() },
                        actual: value.type_().to_string(),
                    });
                }
                // if unsafe { (*array).main.type_ } != value.type_() {
                //     return self.err(TypeError::TypeMisMatch {
                //         expected: unsafe { (*array).main.type_.to_string() },
                //         actual: value.type_().to_string(),
                //     });
                // }
                let array = unsafe { &mut (*array) };
                array.values.push(value);
            }
            ArrayMethod::Pop => {
                self.pop();
                if arg_count != 0 {
                    return self.err(TypeError::ArityMisMatch {
                        name: "pop".to_string(),
                        expected: 0,
                        actual: arg_count,
                    });
                }
                let array = unsafe { &mut (*array) };
                let value = array.values.pop();
                match value {
                    Some(value) => self.push_to_stack(value),
                    None => return self.err(IndexError::IndexOutOfRange { index: 0, len: 0 }),
                }
            }
            ArrayMethod::Get => {
                self.pop();
                if arg_count != 1 {
                    return self.err(TypeError::ArityMisMatch {
                        name: "get".to_string(),
                        expected: 1,
                        actual: arg_count,
                    });
                }
                let index = self.pop();
                let array = unsafe { (*bound_arr_method).array };
                let len = unsafe { (*array).values.len() };
                if index.is_number() && index.as_number() as usize >= len {
                    return self.err(IndexError::IndexOutOfRange {
                        index: index.as_number() as usize,
                        len: (len - 1),
                    });
                }
                let value = unsafe { (*array).values.get_unchecked(index.as_number() as usize) };
                self.push_to_stack(*value);
            }
            ArrayMethod::Len => {
                self.pop();
                if arg_count != 0 {
                    return self.err(TypeError::ArityMisMatch {
                        name: "len".to_string(),
                        expected: 0,
                        actual: arg_count,
                    });
                }
                let array = unsafe { (*bound_arr_method).array };
                let len = unsafe { (*array).values.len() } as f64;
                self.push_to_stack(Value::from(len));
            }
            ArrayMethod::Type => {
                self.pop();
                if arg_count != 0 {
                    return self.err(TypeError::ArityMisMatch {
                        name: "type".to_string(),
                        expected: 0,
                        actual: arg_count,
                    });
                }
                let array = unsafe { (*bound_arr_method).array };
                let type_ = unsafe { ((*array).main.type_).to_string() };
                let name = self.alloc(type_);
                self.push_to_stack(name.into());
            }
            _ => todo!(),
        }
        Ok(())
    }

    fn call_native(&mut self, native: *mut ObjectNative, arg_count: usize) -> Result<()> {
        self.pop();
        let value = match { unsafe { (*native).native } } {
            Native::Clock => {
                if arg_count != 0 {
                    return self.err(TypeError::ArityMisMatch {
                        name: "clock".to_string(),
                        expected: 0,
                        actual: arg_count,
                    });
                }

                let time = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs_f64();
                Value::from(time)
            }
            Native::RandomNumber => {
                if arg_count != 0 {
                    return self.err(TypeError::ArityMisMatch {
                        name: "random_number".to_string(),
                        expected: 0,
                        actual: arg_count,
                    });
                }

                let number = rand::thread_rng().gen_range(1..=100) as f64;
                Value::from(number)
            }
        };
        self.push_to_stack(value);
        Ok(())
    }

    // fn call_array_method(&mut self, array: *mut ArrayObject, arg_count: usize) -> Result<()> {
    //     let method = self.read_constant();
    //     let method = unsafe { method.as_object().string };
    //     match unsafe { (*array).methods.get(&method) } {
    //         Some(&method) => self.call_closure(method, arg_count),
    //         None => self.err(AttributeError::NoSuchAttribute {
    //             type_: unsafe { (*(*array).main.type_).to_string() },
    //             name: unsafe { (*method).value.to_string() },
    //         }),
    //     }
    // }

    fn call_bound_method(
        &mut self,
        method: *mut BoundMethodObject,
        arg_count: usize,
    ) -> Result<()> {
        unsafe { *self.peek(arg_count) = (*method).receiver.into() };
        self.call_closure(unsafe { (*method).method }, arg_count)
    }

    fn call_struct(&mut self, cstruct: *mut StructObject, arg_count: usize) -> Result<()> {
        let instance = self.alloc(InstanceObject::new(cstruct));
        unsafe { *self.peek(arg_count) = Value::from(instance) };

        match unsafe { (*cstruct).methods.get(&self.struct_init_method) } {
            Some(&method) => self.call_closure(method, arg_count),
            None if arg_count != 0 => self.err(TypeError::ArityMisMatch {
                expected: 0,
                actual: arg_count,
                name: unsafe { (*(*(*cstruct).name).value).to_string() },
            }),
            None => Ok(()),
        }
    }

    fn call_closure(&mut self, closure: *mut ClosureObject, arg_count: usize) -> Result<()> {
        let function = unsafe { &mut *(*closure).function };
        if arg_count != (*function).arity_count.into() {
            return self.err(TypeError::ArityMisMatch {
                expected: (*function).arity_count.into(),
                actual: arg_count,
                name: unsafe { (*(*function).name).value }.to_string(),
            });
        }
        if self.frames.len() == FRAMES_MAX {
            return self.err(OverflowError::StackOverflow);
        }
        let frame = CallFrame {
            closure,
            ip: (*function).chunk.code.as_ptr(),
            stack: self.peek(arg_count as usize),
        };
        unsafe {
            self.frames
                .push_unchecked(mem::replace(&mut self.frame, frame))
        };
        Ok(())
    }

    fn capture_upvalue(&mut self, location: Value) -> *mut UpvalueObject {
        match self
            .open_upvalues
            .iter()
            .find(|&&upvalue| unsafe { (*upvalue).value } == location)
        {
            Some(&upvalue) => upvalue,
            None => {
                let upvalue = self.alloc(UpvalueObject::new(location));
                self.open_upvalues.push(upvalue);
                upvalue
            }
        }
    }

    fn greater(&mut self) -> Result<()> {
        self.binary_op_number(|a, b| Value::from(a > b))
    }

    fn greater_equal(&mut self) -> Result<()> {
        self.binary_op_number(|a, b| Value::from(a >= b))
    }

    fn less(&mut self) -> Result<()> {
        self.binary_op_number(|a, b| Value::from(a < b))
    }

    fn less_equal(&mut self) -> Result<()> {
        self.binary_op_number(|a, b| Value::from(a <= b))
    }

    fn loop_(&mut self) -> Result<()> {
        let offset = self.read_u16() as usize;
        self.frame.ip = unsafe { self.frame.ip.sub(offset) };
        Ok(())
    }

    fn jump(&mut self) -> Result<()> {
        let offset = self.read_u16() as usize;
        self.frame.ip = unsafe { self.frame.ip.add(offset) };
        Ok(())
    }

    fn jump_if_false(&mut self) -> Result<()> {
        let offset = self.read_u16() as usize;
        let value = self.peek(0);
        if !unsafe { *value }.to_bool() {
            self.frame.ip = unsafe { self.frame.ip.add(offset) };
        }
        Ok(())
    }

    fn get_local(&mut self) -> Result<()> {
        let stack_idx = self.read_u8() as usize;
        let local = unsafe { *self.frame.stack.add(stack_idx) };
        self.push_to_stack(local);
        Ok(())
    }

    fn set_local(&mut self) -> Result<()> {
        let stack_idx = self.read_u8() as usize;
        let local = unsafe { self.frame.stack.add(stack_idx) };
        let value = self.peek(0);
        unsafe { *local = *value };
        Ok(())
    }

    fn set_global(&mut self) -> Result<()> {
        let name = unsafe { self.read_constant().as_object().string };
        let value = unsafe { *self.peek(0) };
        match self.globals.entry(name) {
            Entry::Occupied(mut entry) => {
                entry.insert(value);
            }
            Entry::Vacant(_) => todo!(),
        }
        Ok(())
    }

    fn get_global(&mut self) -> Result<()> {
        let name = unsafe { self.read_constant().as_object().string };
        let value = self.globals.get(&name).unwrap();
        self.push_to_stack(*value);
        Ok(())
    }

    fn define_global(&mut self) -> Result<()> {
        let name = unsafe { self.read_constant().as_object().string };
        let value = self.pop();
        self.globals.insert(name, value);
        Ok(())
    }

    fn op_nil(&mut self) -> Result<()> {
        self.push_to_stack(Value::NIL);
        Ok(())
    }

    fn op_true(&mut self) -> Result<()> {
        self.push_to_stack(Value::TRUE);
        Ok(())
    }

    fn op_false(&mut self) -> Result<()> {
        self.push_to_stack(Value::FALSE);
        Ok(())
    }

    fn push_to_stack(&mut self, value: Value) {
        unsafe { *self.stack_top = value };
        self.stack_top = unsafe { self.stack_top.add(1) };
    }

    fn pop(&mut self) -> Value {
        self.stack_top = unsafe { self.stack_top.sub(1) };
        unsafe { *self.stack_top }
    }

    fn peek(&self, distance: usize) -> *mut Value {
        unsafe { self.stack_top.sub(distance + 1) }
    }

    fn read_constant(&mut self) -> value::Value {
        let index = self.read_u8() as usize;
        let function = unsafe { (*self.frame.closure).function };
        *unsafe { (*function).chunk.constants.get_unchecked(index) }
    }

    fn read_u8(&mut self) -> u8 {
        let byte = unsafe { *self.frame.ip };
        self.frame.ip = unsafe { self.frame.ip.add(1) };
        byte
    }

    fn read_u16(&mut self) -> u16 {
        let byte1 = self.read_u8();
        let byte2 = self.read_u8();
        (byte1 as u16) << 8 | (byte2 as u16)
    }

    fn modulo(&mut self) -> Result<()> {
        let b = self.pop();
        let a = self.pop();

        if a.is_number() && b.is_number() {
            self.push_to_stack((a.as_number() % b.as_number()).into());
            return Ok(());
        }
        self.err(TypeError::UnsupportedOperandInfix {
            op: "%".to_string(),
            lt_type: a.type_().to_string(),
            rt_type: b.type_().to_string(),
        })
    }

    fn op_concat(&mut self) -> Result<()> {
        let b = self.pop();
        let a = self.pop();

        let a = a.as_object();
        let b = b.as_object();

        if a.type_() == ObjectType::String && b.type_() == ObjectType::String {
            let result = unsafe { [(*a.string).value, (*b.string).value] }.concat();
            let result = Value::from(self.alloc(result));
            self.push_to_stack(result);
            return Ok(());
        }

        self.err(TypeError::UnsupportedOperandInfix {
            op: "+".to_string(),
            lt_type: a.type_().to_string(),
            rt_type: b.type_().to_string(),
        })
    }

    fn op_binary_add(&mut self) -> Result<()> {
        let b = self.pop();
        let a = self.pop();

        if a.is_number() && b.is_number() {
            self.push_to_stack((a.as_number() + b.as_number()).into());
            return Ok(());
        }

        self.err(TypeError::UnsupportedOperandInfix {
            op: "+".to_string(),
            lt_type: a.type_().to_string(),
            rt_type: b.type_().to_string(),
        })
    }

    fn sub(&mut self) -> Result<()> {
        self.binary_op_number(|a, b| Value::from(a - b))
    }

    fn mul(&mut self) -> Result<()> {
        self.binary_op_number(|a, b| Value::from(a * b))
    }

    fn div(&mut self) -> Result<()> {
        self.binary_op_number(|a, b| Value::from(a / b))
    }

    fn binary_op_number(&mut self, op: fn(f64, f64) -> Value) -> Result<()> {
        let b = self.pop();
        let a = self.pop();

        if a.is_number() && b.is_number() {
            let value = op(a.as_number(), b.as_number());
            self.push_to_stack(value);
            return Ok(());
        }
        self.err(TypeError::UnsupportedOperandInfix {
            op: "+".to_string(),
            lt_type: a.type_().to_string(),
            rt_type: b.type_().to_string(),
        })
    }

    fn equal(&mut self) -> Result<()> {
        let rhs = self.pop();
        let lhs = self.pop();
        self.push_to_stack((rhs == lhs).into());
        Ok(())
    }

    fn not_equal(&mut self) -> Result<()> {
        let rhs = self.pop();
        let lhs = self.pop();

        self.push_to_stack((rhs != lhs).into());
        Ok(())
    }

    fn negate(&mut self) -> Result<()> {
        let value: value::Value = self.pop();
        self.push_to_stack(Value::from(-(value.as_number())));
        Ok(())
    }

    fn alloc<T>(&mut self, object: impl CeAlloc<T>) -> T {
        // if GLOBAL.allocated_bytes() > self.next_gc {
        self.gc();
        // }
        let allc = self.allocator.alloc(object);
        allc
    }

    fn gc(&mut self) {
        println!("--- gc start");
        let mut stack_ptr = self.stack_top;
        while stack_ptr < self.stack.as_mut_ptr() {
            self.allocator.mark(unsafe { *stack_ptr });
            stack_ptr = unsafe { stack_ptr.add(1) };
        }

        for (&name, &value) in &self.globals {
            self.allocator.mark(name);
            self.allocator.mark(value);
        }

        self.allocator.mark(self.frame.closure);

        for frame in &self.frames {
            self.allocator.mark(frame.closure);
        }

        for upvalue in &self.open_upvalues {
            self.allocator.mark(*upvalue);
        }

        self.allocator.trace();
        self.allocator.sweep();

        self.next_gc = GLOBAL.allocated_bytes() * 2;

        println!("--- gc end");
    }

    fn err(&self, err: impl Into<Error>) -> Result<()> {
        let function = unsafe { (*self.frame.closure).function };
        let idx = unsafe { self.frame.ip.offset_from((*function).chunk.code.as_ptr()) } as usize;
        let span = unsafe { (*function).chunk.spans[idx - 1].clone() };
        Err((err.into(), span))
    }
}

#[derive(Debug)]
pub struct CallFrame {
    closure: *mut ClosureObject,
    ip: *const u8,
    stack: *mut Value,
}
