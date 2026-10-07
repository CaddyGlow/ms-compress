// SPDX-License-Identifier: LGPL-2.1-or-later
//! Fallible creation and capacity checks for ordinary Rust vectors.

use alloc::collections::TryReserveError;
use alloc::vec::Vec;

pub(crate) fn filled_vec<T: Clone>(size: usize, value: T) -> Result<Vec<T>, TryReserveError> {
    let mut values = Vec::new();
    values.try_reserve_exact(size)?;
    values.resize(size, value);
    Ok(values)
}

// Reused codec workspaces must never grow or allocate during block processing.
pub(crate) fn ensure_capacity<T>(values: &Vec<T>, additional: usize) -> Result<(), ()> {
    if additional <= values.capacity() - values.len() {
        Ok(())
    } else {
        Err(())
    }
}

pub(crate) fn resize_within_capacity<T: Clone>(
    values: &mut Vec<T>,
    size: usize,
    value: T,
) -> Result<(), ()> {
    if size > values.capacity() {
        return Err(());
    }
    values.resize(size, value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_bounds_reject_growth_and_allow_reuse() {
        let mut values = filled_vec(4, 7u8).unwrap();
        let capacity = values.capacity();
        let pointer = values.as_ptr();
        assert!(ensure_capacity(&values, usize::MAX).is_err());
        assert!(resize_within_capacity(&mut values, capacity + 1, 0).is_err());
        assert_eq!(values, [7; 4]);
        resize_within_capacity(&mut values, 1, 0).unwrap();
        ensure_capacity(&values, 3).unwrap();
        values.extend_from_slice(&[1, 2, 3]);
        assert_eq!(values, [7, 1, 2, 3]);
        assert_eq!(values.capacity(), capacity);
        assert_eq!(values.as_ptr(), pointer);
    }
}
