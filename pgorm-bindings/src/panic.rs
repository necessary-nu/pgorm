//! A panic in a binding's work on the runtime, reported as the failure no
//! input should cause.

use std::any::Any;

use crate::Failure;

/// What a panic's payload says: a `panic!` with a literal message carries a
/// `&str`, one with a formatted message a `String`.
fn message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic with no message")
}

/// The `InternalError` a panic in work on the runtime is raised as, every
/// binding's alike (`bindings.errors`).
// [spec:pgorm:req:bindings.errors+6]
pub fn panicked(payload: &(dyn Any + Send)) -> Failure {
    Failure::Internal(format!("pgorm panicked: {}", message(payload)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:pgorm:req:bindings.errors+6/test]    a panic is an InternalError saying what the
    // panic said, from either payload a panic carries
    #[test]
    fn panic_messages_are_read_from_both_payloads() {
        let literal: Box<dyn Any + Send> = Box::new("literal");
        let formatted: Box<dyn Any + Send> = Box::new(String::from("formatted"));
        let opaque: Box<dyn Any + Send> = Box::new(7_u8);
        assert_eq!(
            panicked(literal.as_ref()),
            Failure::Internal("pgorm panicked: literal".to_owned())
        );
        assert_eq!(
            panicked(formatted.as_ref()).message(),
            "pgorm panicked: formatted"
        );
        assert_eq!(
            panicked(opaque.as_ref()).message(),
            "pgorm panicked: a panic with no message"
        );
    }
}
