use pgorm::ValueType;

#[derive(ValueType)]
struct MyInt {
    inner: i32,
}

fn main() {}
