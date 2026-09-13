//! Shared machinery for builtins the engine owns.
//!
//! These types all have the same shape -- an opaque buffer sized from the API dump, a copy
//! constructor, and a destructor -- so the definition is written once here and invoked from the
//! modules that group them by purpose.

use crate::builtin::StringName;
use godot_sys as sys;

/// Resolves a builtin's constructor by index. Index 0 is always the default constructor and
/// index 1 the copy constructor, for every type in this module.
pub(crate) unsafe fn constructor(
    ty: sys::GDExtensionVariantType,
    index: i32,
) -> sys::GDExtensionPtrConstructor {
    let ctor = sys::interface_fn!(variant_get_ptr_constructor)(ty, index);
    assert!(
        ctor.is_some(),
        "engine has no constructor {index} for this builtin"
    );
    ctor
}

/// Resolves the engine's evaluator for operator `op` between two Variant types.
pub(crate) unsafe fn operator_evaluator(
    op: sys::GDExtensionVariantOperator,
    ty_a: sys::GDExtensionVariantType,
    ty_b: sys::GDExtensionVariantType,
) -> sys::GDExtensionPtrOperatorEvaluator {
    let evaluator = sys::interface_fn!(variant_get_ptr_operator_evaluator)(op, ty_a, ty_b);
    assert!(
        evaluator.is_some(),
        "engine has no evaluator for operator {op} on these builtins"
    );
    evaluator
}

/// Evaluates `op` on two operands of Variant type `ty` and takes the result.
///
/// The result goes through [`PtrcallRet`](crate::ptrcall::PtrcallRet) because the evaluator
/// writes it exactly the way a ptrcall writes a return value: it *assigns* into the slot. For a
/// result the engine owns memory for (`String`, `Array`, `Packed*Array`) the slot must therefore
/// start zeroed, never uninitialized, or the engine releases a garbage pointer first -- and
/// `from_ptrcall` is where that rule already lives.
///
/// # Safety
/// `a` and `b` must point to initialized values of type `ty`, and `R` must be the Rust
/// counterpart of the type the engine's operator returns.
pub(crate) unsafe fn evaluate<R: crate::ptrcall::PtrcallRet>(
    op: sys::GDExtensionVariantOperator,
    ty: sys::GDExtensionVariantType,
    a: sys::GDExtensionConstTypePtr,
    b: sys::GDExtensionConstTypePtr,
) -> R {
    let evaluator = operator_evaluator(op, ty, ty).unwrap();
    R::from_ptrcall(|ret| evaluator(a, b, ret))
}

/// Implements Rust's operator traits for a builtin whose memory the engine owns.
///
/// Such a value cannot be compared byte by byte -- two equal strings are two different CowData
/// pointers -- so every operator is the engine's own. Each arm is opt-in because the engine
/// defines a different set of operators per type.
macro_rules! engine_operators {
    ($t:ty, $tag:ident, eq) => {
        impl PartialEq for $t {
            fn eq(&self, other: &Self) -> bool {
                // SAFETY: both operands are initialized values of this type; `==` yields a bool.
                unsafe {
                    $crate::builtin::macros::evaluate(
                        sys::GDExtensionVariantOperator_GDEXTENSION_VARIANT_OP_EQUAL,
                        sys::$tag,
                        self.as_ptr() as sys::GDExtensionConstTypePtr,
                        other.as_ptr() as sys::GDExtensionConstTypePtr,
                    )
                }
            }
        }
    };
    ($t:ty, $tag:ident, eq_hash) => {
        engine_operators!($t, $tag, eq);

        // Only for types whose `==` is reflexive. A container holding a NaN is not equal to
        // itself, which is why the arrays and dictionaries do not get this.
        impl Eq for $t {}

        impl std::hash::Hash for $t {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                // The engine's hash of the contents, not of the handle: two equal values built
                // separately hold different pointers, and must still land in the same bucket.
                state.write_i64(<$t>::hash(self));
            }
        }
    };
    ($t:ty, $tag:ident, ord) => {
        impl PartialOrd for $t {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                // SAFETY: both operands are initialized values of this type; `<` yields a bool.
                let less = |a: &Self, b: &Self| -> bool {
                    unsafe {
                        $crate::builtin::macros::evaluate(
                            sys::GDExtensionVariantOperator_GDEXTENSION_VARIANT_OP_LESS,
                            sys::$tag,
                            a.as_ptr() as sys::GDExtensionConstTypePtr,
                            b.as_ptr() as sys::GDExtensionConstTypePtr,
                        )
                    }
                };

                // Built from the engine's `==` and `<` alone, so the ordering is whatever the
                // engine's is -- including for StringName, whose `<` need not be alphabetical.
                if self == other {
                    Some(std::cmp::Ordering::Equal)
                } else if less(self, other) {
                    Some(std::cmp::Ordering::Less)
                } else if less(other, self) {
                    Some(std::cmp::Ordering::Greater)
                } else {
                    None
                }
            }
        }
    };
    ($t:ty, $tag:ident, add -> $out:ty) => {
        impl std::ops::Add<&$t> for &$t {
            type Output = $out;

            fn add(self, rhs: &$t) -> $out {
                // SAFETY: both operands are initialized values of this type, and `$out` is what
                // the API dump says `+` returns for it.
                unsafe {
                    $crate::builtin::macros::evaluate(
                        sys::GDExtensionVariantOperator_GDEXTENSION_VARIANT_OP_ADD,
                        sys::$tag,
                        self.as_ptr() as sys::GDExtensionConstTypePtr,
                        rhs.as_ptr() as sys::GDExtensionConstTypePtr,
                    )
                }
            }
        }
    };
}

/// Resolves a method on a builtin type, by name and signature hash.
pub(crate) unsafe fn builtin_method(
    ty: sys::GDExtensionVariantType,
    name: &str,
    hash: i64,
) -> sys::GDExtensionPtrBuiltInMethod {
    let name_sn = StringName::new(name);
    let method = sys::interface_fn!(variant_get_ptr_builtin_method)(
        ty,
        name_sn.as_ptr(),
        hash as sys::GDExtensionInt,
    );
    assert!(
        method.is_some(),
        "builtin method `{name}` (hash {hash}) not found -- \
         the engine's API does not match the one these bindings were generated from"
    );
    method
}

/// Defines an engine-owned builtin: construction, copying, destruction, and the conversions
/// that let it cross the FFI boundary.
macro_rules! engine_builtin {
    (
        $(#[$meta:meta])*
        $name:ident, $tag:ident, $size_const:ident
    ) => {
        $(#[$meta])*
        #[repr(C)]
        pub struct $name {
            opaque: [u8; sys::builtin_sizes::$size_const],
        }

        impl $name {
            const VARIANT_TYPE: sys::GDExtensionVariantType = sys::$tag;

            /// Constructs an empty value.
            pub fn new() -> Self {
                // SAFETY: the default constructor initializes the whole buffer.
                unsafe {
                    let mut opaque =
                        MaybeUninit::<[u8; sys::builtin_sizes::$size_const]>::uninit();
                    let ctor = constructor(Self::VARIANT_TYPE, 0).unwrap();
                    ctor(
                        opaque.as_mut_ptr() as sys::GDExtensionUninitializedTypePtr,
                        std::ptr::null(),
                    );
                    Self { opaque: opaque.assume_init() }
                }
            }

            pub fn as_ptr(&self) -> sys::GDExtensionConstTypePtr {
                self.opaque.as_ptr() as sys::GDExtensionConstTypePtr
            }

            pub fn as_mut_ptr(&mut self) -> sys::GDExtensionTypePtr {
                self.opaque.as_mut_ptr() as sys::GDExtensionTypePtr
            }

            /// Copies a value the engine owns.
            ///
            /// # Safety
            /// `ptr` must point to an initialized value of this type.
            pub unsafe fn from_sys_copy(ptr: sys::GDExtensionConstTypePtr) -> Self {
                let mut opaque = MaybeUninit::<[u8; sys::builtin_sizes::$size_const]>::uninit();
                let ctor = constructor(Self::VARIANT_TYPE, 1).unwrap();
                let args: [sys::GDExtensionConstTypePtr; 1] = [ptr];
                ctor(
                    opaque.as_mut_ptr() as sys::GDExtensionUninitializedTypePtr,
                    args.as_ptr(),
                );
                Self { opaque: opaque.assume_init() }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl Clone for $name {
            fn clone(&self) -> Self {
                // SAFETY: `opaque` holds an initialized value of this type.
                unsafe { Self::from_sys_copy(self.as_ptr()) }
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                // SAFETY: the engine's own destructor is the only correct way to release this.
                unsafe {
                    let destructor =
                        sys::interface_fn!(variant_get_ptr_destructor)(Self::VARIANT_TYPE)
                            .expect("engine returned no destructor for this builtin");
                    destructor(self.as_mut_ptr());
                }
            }
        }

        unsafe impl crate::ptrcall::PtrcallArg for $name {}

        unsafe impl crate::ptrcall::PtrcallRet for $name {
            unsafe fn from_ptrcall<F>(call: F) -> Self
            where
                F: FnOnce(sys::GDExtensionTypePtr),
            {
                // Zeroed rather than uninitialized: the engine assigns into the return slot,
                // releasing whatever it finds there first. See `ptrcall` for the full reasoning.
                let mut slot = MaybeUninit::<Self>::zeroed();
                call(slot.as_mut_ptr() as sys::GDExtensionTypePtr);
                slot.assume_init()
            }
        }

        impl ToGodot for $name {
            fn to_variant(&self) -> Variant {
                // SAFETY: `opaque` holds an initialized value of exactly this Variant type.
                unsafe {
                    Variant::from_builtin(
                        Self::VARIANT_TYPE,
                        self.as_ptr() as sys::GDExtensionTypePtr,
                    )
                }
            }
        }

        impl FromGodot for $name {
            fn try_from_variant(variant: &Variant) -> Option<Self> {
                if variant.get_type() != Self::VARIANT_TYPE {
                    return None;
                }
                // SAFETY: the type was just checked.
                unsafe {
                    let mut opaque =
                        MaybeUninit::<[u8; sys::builtin_sizes::$size_const]>::zeroed();
                    variant.to_builtin(
                        Self::VARIANT_TYPE,
                        opaque.as_mut_ptr() as sys::GDExtensionTypePtr,
                    );
                    Some(Self { opaque: opaque.assume_init() })
                }
            }
        }
    };
}
