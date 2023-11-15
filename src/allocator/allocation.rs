use std::hash::BuildHasherDefault;
use std::mem;

use crate::vm::object::{ArrayObject, Object, ObjectType, StringObject};
use crate::vm::value::Value;

use hashbrown::hash_map::RawEntryMut;
use hashbrown::HashMap;
use rustc_hash::FxHasher;

#[derive(Debug)]
pub struct CeAllocation {
    pub strings: HashMap<String, *mut StringObject, BuildHasherDefault<FxHasher>>,
    pub objects: Vec<Object>,
    pub gray_objects: Vec<Object>,
}

impl CeAllocation {
    pub fn new() -> Self {
        Self {
            strings: HashMap::with_hasher(BuildHasherDefault::<FxHasher>::default()),
            objects: Vec::new(),
            gray_objects: Vec::new(),
        }
    }
    pub fn alloc<T>(&mut self, object: impl CeAlloc<T>) -> T {
        object.alloc(self)
    }

    pub fn mark(&mut self, object: impl GcMark) {
        object.mark(self);
    }

    pub fn trace(&mut self) {
        while let Some(object) = self.gray_objects.pop() {
            match unsafe { (*object.main).type_ } {
                ObjectType::BoundMethod => {
                    let method = unsafe { object.bound_method };
                    self.mark(unsafe { (*method).receiver });
                    self.mark(unsafe { (*method).method });
                }
                ObjectType::Struct => {
                    let cstruct = unsafe { object.cstruct };
                    self.mark(unsafe { (*cstruct).name });
                    for (&name, &method) in unsafe { &(*cstruct).methods } {
                        self.mark(name);
                        self.mark(method);
                    }
                }
                ObjectType::Closure => {
                    let closure = unsafe { object.closure };
                    self.mark(unsafe { (*closure).function });
                    for &upvalue in unsafe { &(*closure).upvalues } {
                        self.mark(upvalue);
                    }
                }
                ObjectType::Function => {
                    let function = unsafe { object.function };
                    self.mark(unsafe { (*function).name });
                    for constant in unsafe { &(*function).chunk.constants } {
                        if constant.is_object() {
                            self.mark(constant.as_object());
                        }
                    }
                }
                ObjectType::Instance => {
                    self.mark(unsafe { (*object.instance).struct_ });
                    for (&name, &value) in unsafe { (*object.instance).fields.iter() } {
                        self.mark(name);
                        self.mark(value);
                    }
                }
                ObjectType::Native => {}
                ObjectType::String => {}
                ObjectType::Upvalue => {
                    let upvalue = unsafe { object.upvalue };
                    self.mark(unsafe { (*upvalue).value });
                }
                ObjectType::Array => {
                    let array = unsafe { object.array };
                    for &value in unsafe { &(*array).values } {
                        self.mark(value);
                    }
                }
                ObjectType::BoundArrayMethod => {
                    let method = unsafe { object.bound_array_method };
                    self.mark(unsafe { (*method).array });
                    // self.mark(unsafe { (*method).method });
                }
            }
        }
    }

    pub fn sweep(&mut self) {
        for idx in (0..self.objects.len()).rev() {
            let object = *unsafe { self.objects.get_unchecked(idx) };
            if !mem::take(unsafe { &mut (*object.main).is_marked }) {
                self.objects.swap_remove(idx);
                println!("SWEEPING: {:?}", object);
                object.free();
            }
        }
        // self.strings.retain(|_, &mut string| {
        //     if mem::take(unsafe { &mut (*string).main.is_marked }) {
        //         false
        //     } else {
        //         unsafe { Box::from_raw(string) };
        //         true
        //     }
        // });
    }
}

impl Drop for CeAllocation {
    fn drop(&mut self) {
        // println!("Dropping CeAllocation");
        for object in &self.objects {
            object.free();
        }
        for &string in self.strings.values() {
            unsafe { Box::from_raw(string) };
        }
    }
}

pub trait GcMark {
    fn mark(self, allocator: &mut CeAllocation);
}

impl<T: Into<Object>> GcMark for T {
    fn mark(self, allocator: &mut CeAllocation) {
        let object = self.into();
        if !unsafe { (*object.main).is_marked } {
            // println!("mark {}: {object}", object.type_());
            unsafe { (*object.main).is_marked = true };
            allocator.gray_objects.push(object);
        }
    }
}

impl GcMark for Value {
    fn mark(self, allocator: &mut CeAllocation) {
        if self.is_object() {
            self.as_object().mark(allocator);
        }
    }
}

pub trait CeAlloc<T> {
    fn alloc(self, allocation: &mut CeAllocation) -> T;
}

impl<T> CeAlloc<*mut T> for T
where
    *mut T: Into<Object>,
{
    fn alloc(self, allocation: &mut CeAllocation) -> *mut T {
        let object_ptr = Box::into_raw(Box::new(self));
        let object = object_ptr.into();

        allocation.objects.push(object);
        object_ptr
    }
}

impl<A> CeAlloc<*mut ArrayObject> for A
where
    A: IntoIterator<Item = Value>,
{
    fn alloc(self, allocation: &mut CeAllocation) -> *mut ArrayObject {
        let array = ArrayObject::new(self.into_iter().collect());
        let ptr = Box::into_raw(Box::new(array));
        allocation.objects.push(ptr.into());
        ptr
    }
}

impl<S> CeAlloc<*mut StringObject> for S
where
    S: AsRef<str> + Into<String>,
{
    fn alloc(self, allocation: &mut CeAllocation) -> *mut StringObject {
        match allocation.strings.raw_entry_mut().from_key(self.as_ref()) {
            RawEntryMut::Occupied(entry) => *entry.get(),
            RawEntryMut::Vacant(entry) => {
                let string = self.into();
                let ptr = Box::into_raw(Box::new(StringObject::new(unsafe {
                    mem::transmute(string.as_str())
                })));
                entry.insert(string, ptr);
                ptr
            }
        }
    }
}
