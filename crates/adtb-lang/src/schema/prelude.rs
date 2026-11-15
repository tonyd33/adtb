use super::{PRELUDE, SchemaDefs};
use crate::types::TypeId;

macro_rules! prelude_types {
    ($($field:ident $name:literal),* $(,)?) => {
        /// The prelude types the code names, minted once per build.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Prelude {
            $($field: TypeId,)*
        }

        impl Prelude {
            /// Preconditions: `defs` holds the prelude module with its type names declared.
            pub(super) fn of(defs: &SchemaDefs) -> Prelude {
                let types = &defs.modules[PRELUDE.0 as usize].types;
                Prelude {
                    $($field: *types
                        .get($name)
                        .unwrap_or_else(|| panic!("the prelude declares `{}`", $name)),)*
                }
            }

            $(pub fn $field(&self) -> TypeId {
                self.$field
            })*
        }
    };
}

prelude_types! {
    bool "Bool",
    option "Option",
    result "Result",
    rounding "Rounding",
    overflow "Overflow",
    fault "Fault",
    uuid "Uuid",
    span "Span",
    principal "Principal",
    position "Position",
    error "Error",
    reject_reason "RejectReason",
    limit "Limit",
    origin "Origin",
    outcome "Outcome",
    rejected_row "RejectedRow",
}

macro_rules! prelude_ctors {
    ($($ty:literal { $($name:ident $ctor:literal = $pos:literal),* $(,)? })*) => {
        $($(pub const $name: u32 = $pos;)*)*

        /// Every constant of this module with its name, its type's and its constructor's.
        #[cfg(test)]
        pub(crate) const NAMED: &[(&str, &str, &str, u32)] =
            &[$($((stringify!($name), $ty, $ctor, $name),)*)*];
    };
}

/// The positions of the prelude's constructors, the index a value of each carries.
pub mod ctor {
    prelude_ctors! {
        "Bool" { FALSE "False" = 0, TRUE "True" = 1 }
        "Option" { NONE "None" = 0, SOME "Some" = 1 }
        "Result" { OK "Ok" = 0, ERR "Err" = 1 }
        "Overflow" { CLAMP "Clamp" = 0, REJECT "Reject" = 1 }
        "Rounding" {
            HALF_EVEN "HalfEven" = 0,
            HALF_UP "HalfUp" = 1,
            HALF_DOWN "HalfDown" = 2,
            UP "Up" = 3,
            DOWN "Down" = 4,
            CEILING "Ceiling" = 5,
            FLOOR "Floor" = 6,
        }
        "Fault" {
            DIVISION_BY_ZERO "DivisionByZero" = 0,
            ARITHMETIC_OVERFLOW "ArithmeticOverflow" = 1,
            OUT_OF_RANGE "OutOfRange" = 2,
            NOT_FINITE "NotFinite" = 3,
            NO_SUCH_DATE "NoSuchDate" = 4,
            UNKNOWN_ZONE "UnknownZone" = 5,
            INVALID_TEXT "InvalidText" = 6,
            INVALID_SPAN "InvalidSpan" = 7,
            NEGATIVE_COUNT "NegativeCount" = 8,
            NO_SUCH_ROW "NoSuchRow" = 9,
            FUNCTION_FAILED "FunctionFailed" = 10,
        }
    }

    /// Returns the position of `True` or `False`.
    pub const fn bool(b: bool) -> u32 {
        if b { TRUE } else { FALSE }
    }
}
