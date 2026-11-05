use std::ffi::CString;

use jiff::civil::{date, time};
use pgorm::pgorm_query::{ArrayType, Interval, Value};
use pgorm_python::values::{PyInterval, PyTypeName, PyValue};
use pyo3::prelude::*;

// [spec:pgorm:req:bindings.values/test]
#[test]
fn python_scalars_preserve_rust_variants() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let cases = [
            ("bool", "True", Value::Bool(Some(true))),
            ("i8", "-128", Value::TinyInt(Some(-128))),
            ("i16", "32767", Value::SmallInt(Some(32767))),
            ("i32", "2147483647", Value::Int(Some(i32::MAX))),
            ("i64", "-9223372036854775808", Value::BigInt(Some(i64::MIN))),
            ("u32", "4294967295", Value::Unsigned(Some(u32::MAX))),
            (
                "u64",
                "18446744073709551615",
                Value::BigUnsigned(Some(u64::MAX)),
            ),
            ("f32", "-0.0", Value::Float(Some(-0.0))),
            ("f64", "0.1", Value::Double(Some(0.1))),
            (
                "text",
                "'雪%\\\\'",
                Value::String(Some(Box::new("雪%\\".into()))),
            ),
            ("char", "'雪'", Value::Char(Some('雪'))),
            (
                "bytes",
                "b'\\x00\\xff'",
                Value::Bytes(Some(Box::new(vec![0, 255]))),
            ),
        ];
        for (kind, expression, expected) in cases {
            let expression = CString::new(expression)?;
            let input = py.eval(&expression, None, None)?;
            let actual = py.get_type::<PyValue>().call1((input, kind))?;
            assert_eq!(
                actual.extract::<PyRef<'_, PyValue>>()?.rust_value(),
                &expected
            );
            let inverse = Py::new(py, PyValue::from_rust(expected))?;
            assert!(actual.eq(inverse)?);
        }
        Ok(())
    })
}

// [spec:pgorm:req:bindings.values/test]
#[test]
fn python_interval_is_the_rust_interval() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        for interval in [
            Interval::new(1, -2, 3_000_004),
            Interval::new(i32::MIN, i32::MAX, i64::MIN),
            Interval::new(i32::MAX, i32::MAX, i64::MAX),
        ] {
            let fields = (interval.months, interval.days, interval.microseconds);
            let input = py.get_type::<PyInterval>().call1(fields)?;
            let value = py.get_type::<PyValue>().call1((input, "interval"))?;
            assert_eq!(
                value.extract::<PyRef<'_, PyValue>>()?.rust_value(),
                &Value::from(interval)
            );
            let output = Py::new(py, PyValue::from_rust(Value::from(interval)))?;
            assert!(value.eq(&output)?);
            let read = output.bind(py).getattr("value")?;
            assert_eq!(
                (
                    read.getattr("months")?.extract::<i32>()?,
                    read.getattr("days")?.extract::<i32>()?,
                    read.getattr("microseconds")?.extract::<i64>()?,
                ),
                fields
            );
        }
        let array = Py::new(
            py,
            PyValue::from_rust(Value::array([Interval::new(0, 1, 0)])),
        )?;
        assert_eq!(
            array
                .bind(py)
                .getattr("element_type")?
                .extract::<String>()?,
            "interval"
        );
        assert!(matches!(
            array.borrow(py).rust_value(),
            Value::Array(ArrayType::Interval, Some(_))
        ));
        Ok(())
    })
}

// [spec:pgorm:req:python.values+7/test]
#[test]
fn rust_output_rejects_lost_temporal_precision() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        for inner in [
            Value::Time(Some(Box::new(time(12, 0, 0, 123_456_789)))),
            Value::Date(Some(Box::new(date(0, 1, 1)))),
            Value::Float(Some(f32::from_bits(0x7f800001))),
        ] {
            let value = Py::new(py, PyValue::from_rust(inner))?;
            let error = value.bind(py).getattr("value").err().ok_or_else(|| {
                pyo3::exceptions::PyAssertionError::new_err("lossy conversion succeeded")
            })?;
            assert_eq!(error.get_type(py).name()?, "DecodeError");
            // Inspection still preserves the original Rust payload for diagnosis.
            value.bind(py).call_method0("snapshot")?;
        }
        Ok(())
    })
}

// [spec:pgorm:req:bindings.values/test]
// [spec:pgorm:req:bindings.value-tags/test]
#[test]
fn arrays_and_json_null_keep_rust_identity() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let class = py.get_type::<PyValue>();
        let json_null = class.call_method1("json", (py.None(),))?;
        assert_eq!(
            json_null.extract::<PyRef<'_, PyValue>>()?.rust_value(),
            &Value::Json(Some(Box::new(serde_json::Value::Null)))
        );
        let null = class.call_method1("null", ("json",))?;
        assert_eq!(
            null.extract::<PyRef<'_, PyValue>>()?.rust_value(),
            &Value::Json(None)
        );
        let input = vec![Some(7i32), None];
        let array = class.call_method1("array", ("i32", input))?;
        assert_eq!(
            array.extract::<PyRef<'_, PyValue>>()?.rust_value(),
            &Value::Array(
                ArrayType::Int,
                Some(Box::new(vec![Value::Int(Some(7)), Value::Int(None)]))
            )
        );
        let name = py.get_type::<PyTypeName>().call1(("Mood\"雪",))?;
        let enum_value = class.call1(("calm", &name))?;
        assert_eq!(
            enum_value.extract::<PyRef<'_, PyValue>>()?.rust_value(),
            &Value::String(Some(Box::new("calm".into())))
        );
        let rust_type = name.extract::<PyRef<'_, PyTypeName>>()?.rust_type();
        assert!(!rust_type.is_verbatim());
        assert_eq!(rust_type.name.to_string(), "Mood\"雪");
        Ok(())
    })
}

// [spec:pgorm:def:sql.value.range+5/test]    a range or multirange reaches Python as a
// `pgorm.Range` or `pgorm.Multirange`, tagged with its range type, and converts back unchanged
// [spec:pgorm:req:bindings.values/test]
#[test]
fn rust_ranges_round_trip_through_python() -> PyResult<()> {
    use pgorm::pgorm_query::{Multirange, Range, RangeType};
    use pgorm_python::values::PyRange;
    use std::ops::Bound;

    Python::initialize();
    Python::attach(|py| {
        let null_lower = Value::Range(
            RangeType::Int4,
            Some(Box::new(Range::new(
                Bound::Included(Value::Int(None)),
                Bound::Excluded(Value::Int(Some(5))),
            ))),
        );
        for (inner, kind) in [
            (Value::from(Range::from(1..5)), "int4range"),
            (Value::from(Range::<i64>::Empty), "int8range"),
            (Value::Range(RangeType::Date, None), "daterange"),
            (
                Value::from(
                    [Range::from(1i64..3), Range::Empty]
                        .into_iter()
                        .collect::<Multirange<i64>>(),
                ),
                "int8multirange",
            ),
        ] {
            let value = Py::new(py, PyValue::from_rust(inner.clone()))?;
            let value = value.bind(py);
            assert_eq!(value.getattr("kind")?.extract::<String>()?, kind);
            let python = value.getattr("value")?;
            let back = py.get_type::<PyValue>().call1((python, kind))?;
            assert_eq!(back.extract::<PyRef<'_, PyValue>>()?.rust_value(), &inner);
        }
        // A NULL bound is no bound: it reaches Python as an unbounded side.
        let unbounded = Py::new(py, PyValue::from_rust(null_lower))?;
        let range = unbounded.bind(py).getattr("value")?;
        assert!(range.is_exact_instance_of::<PyRange>());
        assert!(range.getattr("lower_inf")?.extract::<bool>()?);
        assert_eq!(range.getattr("bounds")?.extract::<String>()?, "()");
        Ok(())
    })
}

// [spec:pgorm:req:bindings.created-ranges+3/test]    a created range's Python value is the Value its Rust
// newtype converts into, written as the newtype writes itself, and reads back as the range
#[test]
fn created_ranges_match_their_rust_newtypes() -> PyResult<()> {
    use pgorm::{
        CreatedRange,
        entity::prelude::*,
        pgorm_query::{Multirange, Query, Range},
    };
    use pgorm_python::expressions::PyExpr;

    #[derive(Clone, Debug, PartialEq, CreatedRange)]
    #[pgorm(range_name = "Float Range", schema_name = "measure")]
    struct FloatRange(Range<f64>);

    #[derive(Clone, Debug, PartialEq, CreatedRange)]
    #[pgorm(multirange_name = "slot_multirange")]
    struct Slots(Multirange<i32>);

    Python::initialize();
    Python::attach(|py| {
        let pgorm = PyModule::new(py, "pgorm")?;
        pgorm_python::install(&pgorm, Default::default())?;
        let globals = pyo3::types::PyDict::new(py);
        globals.set_item("p", &pgorm)?;
        let cases: [(&str, Value, pgorm::pgorm_query::SimpleExpr); 2] = [
            (
                "p.Value(p.Range(1.5, None), p.CreatedRange('Float Range', 'f64', schema='measure'))",
                FloatRange(Range::from(1.5..)).into(),
                FloatRange(Range::from(1.5..)).into_expr(),
            ),
            (
                "p.Value(p.Multirange([p.Range(5, 8), p.Range(1, 3, '[]')]), p.CreatedMultirange('slot_multirange', 'i32'))",
                Slots(Multirange::from(vec![
                    Range::from(5..8),
                    Range::from(1..=3),
                ]))
                .into(),
                Slots(Multirange::from(vec![
                    Range::from(5..8),
                    Range::from(1..=3),
                ]))
                .into_expr(),
            ),
        ];
        for (source, expected, written) in cases {
            let value = py.eval(&CString::new(source)?, Some(&globals), None)?;
            assert_eq!(
                value.extract::<PyRef<'_, PyValue>>()?.rust_value(),
                &expected
            );
            let bound = pgorm.getattr("bind")?.call1((&value,))?;
            assert_eq!(bound.extract::<PyRef<'_, PyExpr>>()?.inner, written);
            let compiled = bound.call_method0("inspect")?;
            assert_eq!(
                compiled.getattr("sql")?.extract::<String>()?,
                Query::select().expr(written).build().0
            );
            let kind = value.getattr("created_type")?;
            let back = py
                .get_type::<PyValue>()
                .call1((value.getattr("value")?, kind))?;
            assert_eq!(
                back.extract::<PyRef<'_, PyValue>>()?.rust_value(),
                &expected
            );
        }
        Ok(())
    })
}
