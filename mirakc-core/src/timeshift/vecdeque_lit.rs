// Took-From: https://github.com/bluss/maplit/pull/31/files#diff-b1a35a68f14e696205874893c07fd24fdb88882b47c23cc0e0c80a30c7d53759
// SPDX-License-Identifier: MIT OR Apache-2.0

macro_rules! vecdeque {
    (@single $($x:tt)*) => (());
    (@count $($rest:expr),*) => (<[()]>::len(&[$(vecdeque!(@single $rest)),*]));

    ($($value:expr,)+) => { vecdeque!($($value),+) };
    ($($value:expr),*) => {
        {
            let _cap = vecdeque!(@count $($value),*);
            let mut _map = ::std::collections::VecDeque::with_capacity(_cap);
            $(
                _map.push_back($value);
            )*
            _map
        }
    };
    ($value:expr;$count:expr) => {
        {
            let c = $count;
            let mut _map = ::std::collections::VecDeque::with_capacity(c);
            for _ in 0..c {
                _map.push_back($value);
            }
            _map
        }
    };
}
