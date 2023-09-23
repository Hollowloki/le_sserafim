use crate::allocator::allocation::{CeAlloc, CeAllocation};

pub mod chunk;
pub mod compiler;
pub mod object;
pub mod op;
pub mod value;

#[derive(Debug)]
pub struct VM {
    chunk: chunk::Chunk,
    ip: usize,
    stack: Vec<value::Value>,
    allocator: CeAllocation,
}

impl VM {
    pub fn new(chunk: chunk::Chunk) -> VM {
        VM {
            chunk,
            ip: 0,
            stack: Vec::new(),
            allocator: CeAllocation::new(),
        }
    }

    pub fn run(&mut self) {
        loop {
            println!("{:?}", self.stack);
            match self.read_byte() {
                op::PRINT => {
                    let value: value::Value = self.stack.pop().unwrap();
                    println!("{}", value);
                }
                op::ADD => self.add(),
                op::SUB => self.sub(),
                op::MUL => self.mul(),
                op::DIV => self.div(),
                op::EQUAL => self.equal(),
                op::NOT_EQUAL => self.not_equal(),
                op::NEG => self.negate(),
                op::CECILE_CONSTANT => {
                    let constant = self.read_constant();
                    println!(" pushing constant {:?}", constant);
                    self.stack.push(constant);
                }
                op::TRUE => self.op_true(),
                op::FALSE => self.op_false(),
                op::NIL => self.op_nil(),
                op::RETURN => {
                    self.stack.pop();
                    return;
                }
                _ => todo!(),
            }
        }
    }

    fn op_nil(&mut self) {
        self.stack.push(value::Value::Number(0.0));
    }

    fn op_true(&mut self) {
        self.stack.push(true.into());
    }

    fn op_false(&mut self) {
        self.stack.push(false.into());
    }

    fn read_constant(&mut self) -> value::Value {
        let index = self.read_byte() as usize;
        let constant = self.chunk.constants[index].clone();
        constant
    }

    fn read_byte(&mut self) -> u8 {
        let byte = self.chunk.code[self.ip];
        self.ip += 1;
        byte
    }

    fn add(&mut self) {
        let rhs = self.stack.pop().unwrap();
        let lhs = self.stack.pop().unwrap();
        println!(" adding {} + {}", lhs, rhs);
        self.stack.push(lhs.add(rhs));
    }

    fn sub(&mut self) {
        let rhs = self.stack.pop().unwrap();
        let lhs = self.stack.pop().unwrap();
        println!(" subtracting {} - {}", lhs, rhs);
        self.stack.push(lhs.sub(rhs));
    }

    fn mul(&mut self) {
        let rhs = self.stack.pop().unwrap();
        let lhs = self.stack.pop().unwrap();
        println!(" multiplying {} * {}", lhs, rhs);
        self.stack.push(lhs.mul(rhs));
    }

    fn div(&mut self) {
        let rhs = self.stack.pop().unwrap();
        let lhs = self.stack.pop().unwrap();
        println!(" dividing {} / {}", lhs, rhs);
        self.stack.push(lhs.div(rhs));
    }

    fn equal(&mut self) {
        let rhs = self.stack.pop().unwrap();
        let lhs = self.stack.pop().unwrap();
        println!(" comparing {} == {}", lhs, rhs);
        self.stack.push((lhs == rhs).into());
    }

    fn not_equal(&mut self) {
        let rhs = self.stack.pop().unwrap();
        let lhs = self.stack.pop().unwrap();
        println!(" comparing {} != {}", lhs, rhs);

        self.stack.push((lhs != rhs).into());
    }

    fn negate(&mut self) {
        let value: value::Value = self.stack.pop().unwrap();
        println!(" negating {}", value);
        // Value to f64
        self.stack.push(value.neg());
    }

    fn alloc<T>(&mut self, object: impl CeAlloc<T>) -> T {
        self.allocator.alloc(object)
    }
}
