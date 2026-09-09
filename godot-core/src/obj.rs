use crate::ptrcall::MethodBind;
use godot_sys as sys;
use std::marker::PhantomData;
use std::sync::OnceLock;

/// `RefCounted`'s lifetime methods, resolved once.
///
/// These live here rather than in the generated bindings because `Gd`'s `Clone` and `Drop` need
/// them, and `godot-core` cannot depend on the generated crate without a dependency cycle. The
/// hashes come from `extension_api.json` via `godot-sys`, not from hand-copied constants.
struct RefCountMethods {
    init_ref: MethodBind,
    reference: MethodBind,
    unreference: MethodBind,
}

fn refcount_methods() -> &'static RefCountMethods {
    static METHODS: OnceLock<RefCountMethods> = OnceLock::new();
    METHODS.get_or_init(|| unsafe {
        RefCountMethods {
            init_ref: MethodBind::resolve(
                "RefCounted",
                "init_ref",
                sys::method_hashes::REFCOUNTED_INIT_REF,
            ),
            reference: MethodBind::resolve(
                "RefCounted",
                "reference",
                sys::method_hashes::REFCOUNTED_REFERENCE,
            ),
            unreference: MethodBind::resolve(
                "RefCounted",
                "unreference",
                sys::method_hashes::REFCOUNTED_UNREFERENCE,
            ),
        }
    })
}

/// An engine class, either built into Godot or registered by an extension.
///
/// # Safety
/// `CLASS_NAME` must name a class that actually exists in ClassDB, and the implementing type
/// must only ever be used behind a [`Gd`].
pub unsafe trait GodotObject: 'static {
    const CLASS_NAME: &'static str;

    /// Whether Godot reference-counts this class (`RefCounted` and its descendants).
    const IS_REFCOUNTED: bool;
}

/// Marks that `Self` inherits from `Base`, directly or transitively.
///
/// Generated for every class/ancestor pair, which is what makes [`Gd::upcast_ref`] safe: the
/// compiler checks the relationship the engine's hierarchy already guarantees.
///
/// # Safety
/// Only implement when `Self` really does inherit `Base` in Godot's class hierarchy.
pub unsafe trait Inherits<Base: GodotObject>: GodotObject {}

/// A handle to a Godot object.
///
/// Replaces the Godot 3 `Ref<T, Ownership>` / `TRef` pair and its Unique/Shared/ThreadLocal
/// markers: GDExtension has no thread-local object API, so the three-state ownership model has
/// no counterpart here.
///
/// Lifetime today is manual -- call [`Gd::free`] for manually-managed classes. Automatic
/// reference counting for `RefCounted` descendants arrives together with the generated
/// `reference`/`unreference` bindings.
#[repr(transparent)]
pub struct Gd<T: GodotObject> {
    ptr: sys::GDExtensionObjectPtr,
    _marker: PhantomData<*mut T>,
}

impl<T: GodotObject> Gd<T> {
    /// Wraps a raw engine object pointer.
    ///
    /// # Safety
    /// `ptr` must be a live object of class `T` (or a subclass), and the caller must not free it
    /// while this handle is in use.
    pub unsafe fn from_obj_ptr(ptr: sys::GDExtensionObjectPtr) -> Option<Self> {
        if ptr.is_null() {
            None
        } else {
            Some(Self {
                ptr,
                _marker: PhantomData,
            })
        }
    }

    pub fn as_obj_ptr(&self) -> sys::GDExtensionObjectPtr {
        self.ptr
    }

    /// Constructs a new instance through ClassDB.
    ///
    /// For reference-counted classes the initial reference is taken here, so the returned
    /// handle owns exactly one count.
    pub fn new() -> Option<Self> {
        unsafe {
            let name = crate::builtin::StringName::new(T::CLASS_NAME);
            let ptr = sys::interface_fn!(classdb_construct_object2)(name.as_ptr());
            let gd = Self::from_obj_ptr(ptr)?;

            if T::IS_REFCOUNTED {
                // ClassDB hands back a RefCounted with a zero count; `init_ref` takes the first.
                let _: bool = refcount_methods().init_ref.ptrcall(ptr, &[]);
            }

            Some(gd)
        }
    }

    /// Destroys a manually-managed object.
    ///
    /// # Safety
    /// Only for classes Godot does not reference-count, and only when no other handle to the
    /// object remains.
    pub unsafe fn free(self) {
        assert!(
            !T::IS_REFCOUNTED,
            "{} is reference-counted; it must not be freed manually",
            T::CLASS_NAME
        );
        sys::interface_fn!(object_destroy)(self.ptr);
        // `Drop` is a no-op for manually-managed classes, so letting `self` fall out of scope
        // here is correct; nothing double-frees.
    }

    /// Reinterprets this handle as a base class.
    ///
    /// # Safety
    /// `Base` must actually be a base class of `T`.
    pub unsafe fn upcast_unchecked<Base: GodotObject>(self) -> Gd<Base> {
        // The reference count is transferred, not duplicated: suppress this handle's `Drop`.
        let this = std::mem::ManuallyDrop::new(self);
        Gd {
            ptr: this.ptr,
            _marker: PhantomData,
        }
    }

    /// Views this handle as one of its base classes, without touching the reference count.
    ///
    /// Zero-cost: `Gd<T>` is `repr(transparent)` around the object pointer, so the base handle
    /// has the same representation. Borrowing rather than consuming is what lets a `Gd<Resource>`
    /// be passed to a method declared on `RefCounted`.
    pub fn upcast_ref<Base>(&self) -> &Gd<Base>
    where
        Base: GodotObject,
        T: Inherits<Base>,
    {
        // SAFETY: both sides are `repr(transparent)` over the same pointer, and `Inherits`
        // guarantees the object really is a `Base`.
        unsafe { &*(self as *const Gd<T> as *const Gd<Base>) }
    }

    /// Attempts to view this object as `Target`, checking the engine's own class hierarchy.
    pub fn try_cast<Target: GodotObject>(&self) -> Option<Gd<Target>> {
        unsafe {
            let name = crate::builtin::StringName::new(Target::CLASS_NAME);
            let tag = sys::interface_fn!(classdb_get_class_tag)(name.as_ptr());
            if tag.is_null() {
                return None;
            }

            let casted = sys::interface_fn!(object_cast_to)(self.ptr, tag);
            let result = Gd::<Target>::from_obj_ptr(casted)?;

            if Target::IS_REFCOUNTED {
                // A second handle to the same object owns a second count.
                let _: bool = refcount_methods().reference.ptrcall(casted, &[]);
            }

            Some(result)
        }
    }

    /// The engine-wide id of this object, stable while the object lives.
    pub fn instance_id(&self) -> u64 {
        unsafe { sys::interface_fn!(object_get_instance_id)(self.ptr) }
    }
}

/// Reaching a class's methods through `Gd`.
///
/// The marker types are zero-sized and live at the same address as the `Gd` they were reached
/// through, so a method holding `&self` can recover the object pointer. Each class also derefs
/// to its base, which is what makes an inherited method callable without naming the base class.
impl<T: GodotObject> std::ops::Deref for Gd<T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: `Gd` is `repr(transparent)` over the pointer and `T` is zero-sized, so the
        // reference is in bounds and carries no data of its own.
        unsafe { &*(self as *const Gd<T> as *const T) }
    }
}

/// Recovers the object pointer from a method's `&self`.
///
/// # Safety
/// `this` must be a reference obtained by dereferencing a live `Gd`, which is the only way the
/// generated methods are reachable.
#[doc(hidden)]
pub unsafe fn obj_ptr_from_ref<T>(this: &T) -> sys::GDExtensionObjectPtr {
    // The marker sits at the `Gd`'s address; read the pointer back out of it.
    *(this as *const T as *const sys::GDExtensionObjectPtr)
}

impl<T: GodotObject> Clone for Gd<T> {
    fn clone(&self) -> Self {
        if T::IS_REFCOUNTED {
            // SAFETY: `ptr` is a live RefCounted; taking a count is what makes the copy valid.
            unsafe {
                let _: bool = refcount_methods().reference.ptrcall(self.ptr, &[]);
            }
        }

        Self {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}

impl<T: GodotObject> Drop for Gd<T> {
    fn drop(&mut self) {
        if !T::IS_REFCOUNTED {
            // Manually-managed objects outlive their handles; `free` is the explicit way out.
            return;
        }

        // SAFETY: this handle owns one reference count, released here. `unreference` returns
        // true when the count reached zero and the object must be destroyed.
        unsafe {
            let last: bool = refcount_methods().unreference.ptrcall(self.ptr, &[]);
            if last {
                sys::interface_fn!(object_destroy)(self.ptr);
            }
        }
    }
}

impl<T: GodotObject> crate::builtin::ToGodot for Gd<T> {
    fn to_variant(&self) -> crate::builtin::Variant {
        // An Object Variant holds the object pointer, so the address of this handle is the
        // native representation the constructor expects.
        unsafe {
            crate::builtin::Variant::from_builtin(
                sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_OBJECT,
                &self.ptr as *const sys::GDExtensionObjectPtr as sys::GDExtensionTypePtr,
            )
        }
    }
}

impl<T: GodotObject> crate::builtin::FromGodot for Gd<T> {
    fn try_from_variant(variant: &crate::builtin::Variant) -> Option<Self> {
        if variant.get_type() != sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_OBJECT {
            return None;
        }

        unsafe {
            let mut ptr: sys::GDExtensionObjectPtr = std::ptr::null_mut();
            variant.to_builtin(
                sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_OBJECT,
                &mut ptr as *mut sys::GDExtensionObjectPtr as sys::GDExtensionTypePtr,
            );

            let gd = Self::from_obj_ptr(ptr)?;

            if T::IS_REFCOUNTED {
                // The Variant keeps its own count; this handle needs one of its own.
                let _: bool = refcount_methods().reference.ptrcall(ptr, &[]);
            }

            Some(gd)
        }
    }
}

impl<T: GodotObject> std::fmt::Debug for Gd<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Gd<{}>({:?})", T::CLASS_NAME, self.ptr)
    }
}
