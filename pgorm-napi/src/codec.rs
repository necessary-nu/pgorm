//! The JavaScript half of value conversion, which `lib/values.js` registers.
//!
//! Neon has no `instanceof`, and the classes a value can be an instance of —
//! the module's own `Decimal`, `Range` and the rest, and the runtime's
//! `Temporal` types — are JavaScript's. So the module registers two functions
//! with its instance: `describe`, which names what a JavaScript object is and
//! hands over the fields it holds, and `make`, which builds one from fields.
//! Every judgement about a value — which kind it is, whether it is in range,
//! whether its precision survives — is made in Rust; the two functions only
//! read and build.

use std::sync::{Mutex, PoisonError};

use neon::{prelude::*, thread::LocalKey, types::function::TryIntoArguments};

struct Registered {
    describe: Root<JsFunction>,
    make: Root<JsFunction>,
}

/// Instance-local because the classes belong to one JavaScript realm: a worker
/// thread loading the addon registers its own.
static CODEC: LocalKey<Mutex<Option<Registered>>> = LocalKey::new();

/// `setCodec(describe, make)`: register the functions values are read and
/// built with. `lib/values.js` calls it once as the module loads.
pub(crate) fn set_codec(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    let describe = cx.argument::<JsFunction>(0)?.root(&mut cx);
    let make = cx.argument::<JsFunction>(1)?.root(&mut cx);
    let slot = CODEC.get_or_init(&mut cx, Default::default);
    let previous = slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .replace(Registered { describe, make });
    if let Some(previous) = previous {
        previous.describe.drop(&mut cx);
        previous.make.drop(&mut cx);
    }
    Ok(cx.undefined())
}

/// What `describe` said a JavaScript object is: its tag, and the fields that
/// follow it.
pub(crate) struct Description<'cx> {
    pub(crate) tag: String,
    pub(crate) fields: Vec<Handle<'cx, JsValue>>,
}

impl<'cx> Description<'cx> {
    /// The field at `index`, or `undefined` past the end.
    pub(crate) fn field(&self, cx: &mut Cx<'cx>, index: usize) -> Handle<'cx, JsValue> {
        match self.fields.get(index) {
            Some(field) => *field,
            None => cx.undefined().upcast(),
        }
    }
}

/// The registered functions, as handles valid for one call into the addon.
#[derive(Clone, Copy)]
pub(crate) struct Codec<'cx> {
    describe: Handle<'cx, JsFunction>,
    make: Handle<'cx, JsFunction>,
}

impl<'cx> Codec<'cx> {
    pub(crate) fn get(cx: &mut Cx<'cx>) -> NeonResult<Self> {
        let handles = CODEC.get(cx).and_then(|slot| {
            slot.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
                .map(|registered| {
                    (
                        registered.describe.to_inner(cx),
                        registered.make.to_inner(cx),
                    )
                })
        });
        match handles {
            Some((describe, make)) => Ok(Self { describe, make }),
            None => cx.throw_error(
                "pgorm-napi converts values only when loaded through lib/pgorm.js, which \
                 registers how they are read and built",
            ),
        }
    }

    /// What `value`, an object, is: `None` when it is no object the module
    /// knows.
    pub(crate) fn describe(
        &self,
        cx: &mut Cx<'cx>,
        value: Handle<'cx, JsValue>,
    ) -> NeonResult<Option<Description<'cx>>> {
        let described: Handle<JsValue> = self.describe.bind(cx).arg(value)?.call()?;
        let Ok(array) = described.downcast::<JsArray, _>(cx) else {
            return Ok(None);
        };
        let mut fields = array.to_vec(cx)?;
        if fields.is_empty() {
            return Ok(None);
        }
        let tag = fields
            .remove(0)
            .downcast_or_throw::<JsString, _>(cx)?
            .value(cx);
        Ok(Some(Description { tag, fields }))
    }

    /// Build the JavaScript value `args` describe, the first of them its tag.
    pub(crate) fn make<A: TryIntoArguments<'cx>>(
        &self,
        cx: &mut Cx<'cx>,
        args: A,
    ) -> JsResult<'cx, JsValue> {
        self.make.bind(cx).args(args)?.call()
    }
}
