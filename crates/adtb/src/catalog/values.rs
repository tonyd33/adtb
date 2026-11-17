//! Values of the prelude sums catalog rows hold.

use adtb_lang::schema::prelude::ctor;

use crate::key::Value;

/// Returns `b` as a value of the prelude's `Bool`.
pub fn boolean(b: bool) -> Value {
    Value::Sum {
        ctor: ctor::bool(b),
        fields: Vec::new(),
    }
}

/// Returns the prelude's `None`.
pub fn none() -> Value {
    Value::Sum {
        ctor: ctor::NONE,
        fields: Vec::new(),
    }
}

/// Returns the prelude's `Some v`.
pub fn some(v: Value) -> Value {
    Value::Sum {
        ctor: ctor::SOME,
        fields: vec![v],
    }
}
