// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Enum wire texts declared once: each table gives the text method and, where the enum is read back, its parser.

/// `wire_text!(impl Enum { name[, parse][; ALL] } Variant => "TEXT", ...)` adds the methods to a local enum;
/// `wire_text!(fn Enum { name, parse } ...)` writes free functions for a foreign one.
macro_rules! wire_text {
    (impl $ty:ident { $name:ident $(, $parse:ident)? $(; $all:ident)? } $($variant:ident => $text:literal,)+) => {
        impl $ty {
            $crate::wire::wire_text!(@all [$($all)?] $($variant)+);

            /// The variant's wire text.
            pub const fn $name(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }

            $crate::wire::wire_text!(@parse [$($parse)?] $($variant => $text,)+);
        }
    };
    (fn $ty:ident { $name:ident, $parse:ident } $($variant:ident => $text:literal,)+) => {
        /// The variant's wire text.
        pub const fn $name(value: $ty) -> &'static str {
            match value {
                $($ty::$variant => $text,)+
            }
        }

        /// The variant for its wire text; `None` for a text it does not know.
        pub fn $parse(text: &str) -> Option<$ty> {
            match text {
                $($text => Some($ty::$variant),)+
                _ => None,
            }
        }
    };
    // Optional parts take their name in brackets, so the variant list repeats on its own.
    (@all [] $($variant:ident)+) => {};
    (@all [$all:ident] $($variant:ident)+) => {
        /// Every variant, in declaration order.
        pub const $all: [Self; [$(stringify!($variant)),+].len()] = [$(Self::$variant),+];
    };
    (@parse [] $($variant:ident => $text:literal,)+) => {};
    (@parse [$parse:ident] $($variant:ident => $text:literal,)+) => {
        /// The variant for its wire text; `None` for a text it does not know.
        pub fn $parse(text: &str) -> Option<Self> {
            match text {
                $($text => Some(Self::$variant),)+
                _ => None,
            }
        }
    };
}

pub(crate) use wire_text;
