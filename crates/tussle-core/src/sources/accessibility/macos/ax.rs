//! Thin wrappers around `AXUIElementCopyAttributeValue` for the three
//! attribute shapes we read: opaque element, string, integer, child array.

use std::ffi::c_void;
use std::ptr;

use accessibility_sys::{
    AXError, AXUIElementCopyAttributeValue, AXUIElementGetTypeID, AXUIElementRef,
    kAXChildrenAttribute, kAXErrorSuccess,
};
use core_foundation::ConcreteCFType;
use core_foundation::array::CFArray;
use core_foundation::base::{CFGetTypeID, CFRelease, CFType, CFTypeRef, TCFType};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;

/// Bound strings supplied by another process before converting them into a
/// Rust allocation. Real app and menu names are far below this size.
const MAX_AX_STRING_UTF16_UNITS: isize = 1024;

fn copy_value(element: AXUIElementRef, attribute: &str) -> Option<CFTypeRef> {
    let attr = CFString::new(attribute);
    let mut value: CFTypeRef = ptr::null();
    let err: AXError =
        unsafe { AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value) };
    if err != kAXErrorSuccess {
        if !value.is_null() {
            // SAFETY: defensively balance a create-rule value even if a
            // provider returned it alongside an error status.
            unsafe { CFRelease(value) };
        }
        return None;
    }
    if value.is_null() {
        return None;
    }
    Some(value)
}

fn copy_typed<T: ConcreteCFType>(element: AXUIElementRef, attribute: &str) -> Option<T> {
    let value = copy_value(element, attribute)?;
    // SAFETY: AXUIElementCopyAttributeValue returned a non-null object under
    // Core Foundation's create rule. downcast_into validates its runtime type
    // and releases it automatically when the type does not match.
    let value = unsafe { CFType::wrap_under_create_rule(value) };
    value.downcast_into::<T>()
}

pub(super) fn copy_element(element: AXUIElementRef, attribute: &str) -> Option<AXUIElementRef> {
    let value = copy_value(element, attribute)?;
    // SAFETY: value is a non-null Core Foundation object returned under the
    // create rule. Type IDs may be queried for any valid CF object.
    let is_element = unsafe { CFGetTypeID(value) == AXUIElementGetTypeID() };
    if is_element {
        Some(value as AXUIElementRef)
    } else {
        // SAFETY: balance the create-rule reference on the rejected value.
        unsafe { CFRelease(value) };
        None
    }
}

/// Validate a borrowed child from a CFArray before using it as an AX element.
pub(super) fn borrowed_element(value: *const c_void) -> Option<AXUIElementRef> {
    if value.is_null() {
        return None;
    }
    // SAFETY: CFArray contains CF objects supplied by Accessibility. Querying
    // the runtime ID does not take ownership of the borrowed value.
    let is_element = unsafe { CFGetTypeID(value) == AXUIElementGetTypeID() };
    is_element.then_some(value as AXUIElementRef)
}

pub(super) fn copy_children(element: AXUIElementRef) -> Option<CFArray<*const c_void>> {
    copy_typed(element, kAXChildrenAttribute)
}

pub(super) fn copy_string(element: AXUIElementRef, attribute: &str) -> Option<String> {
    let value: CFString = copy_typed(element, attribute)?;
    if value.char_len() > MAX_AX_STRING_UTF16_UNITS {
        return None;
    }
    Some(value.to_string())
}

pub(super) fn copy_i64(element: AXUIElementRef, attribute: &str) -> Option<i64> {
    copy_typed::<CFNumber>(element, attribute)?.to_i64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_element_rejects_other_core_foundation_types() {
        let string = CFString::new("not an accessibility element");
        assert!(borrowed_element(string.as_CFTypeRef()).is_none());
    }
}
