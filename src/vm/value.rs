use std::fmt::{Debug, Display};

use crate::vm::object::ObjectType;

use super::object::{Object, StringObject};

#[derive(PartialEq, Clone, Copy)]
pub enum Value {
    Number(f64),
    String(*mut StringObject),
}

impl Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(n) => write!(f, "{}", n),
            Value::String(ptr) => unsafe { write!(f, "{}", (**ptr).value) },
        }
    }
}

impl Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(n) => write!(f, "{}", n),
            Value::String(ptr) => write!(f, "{}", unsafe { (**ptr).value }),
        }
    }
}

impl Into<f64> for Value {
    fn into(self) -> f64 {
        match self {
            Value::Number(n) => n,
            _ => todo!(),
        }
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Number(b as i32 as f64)
    }
}

impl Value {
    pub fn add(&self, other: Value) -> Value {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => Value::Number(a + b),
            _ => todo!(),
        }
    }

    pub fn sub(&self, other: Value) -> Value {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => Value::Number(a - b),
            _ => todo!(),
        }
    }

    pub fn mul(&self, other: Value) -> Value {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => Value::Number(a * b),
            _ => todo!(),
        }
    }

    pub fn div(&self, other: Value) -> Value {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => Value::Number(a / b),
            _ => todo!(),
        }
    }

    pub fn neg(&self) -> Value {
        match self {
            Value::Number(n) => Value::Number(-n),
            _ => todo!(),
        }
    }
}
