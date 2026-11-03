// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Enum wire texts declared once: each table gives the text method and, where the enum is read back, its parser.

/// `wire_text!(impl Enum { name[, parse][; ALL] } Variant => "TEXT", ...)` adds the methods to a local enum;
/// `wire_text!(fn Enum { name, parse } ...)` writes free functions for a foreign one.
macro_rules! wire_text {
    (impl $ty:ident { $name:ident $(, $parse:ident)? $(; $all:ident)? } $($variant:ident => $text:literal,)+) => {
        impl $ty {
            $crate::wire::wire_text!(@all [$($all)?] $($variant)+);

            pub const fn $name(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }

            $crate::wire::wire_text!(@parse [$($parse)?] $($variant => $text,)+);
        }
    };
    (fn $ty:ident { $name:ident, $parse:ident } $($variant:ident => $text:literal,)+) => {
        pub const fn $name(value: $ty) -> &'static str {
            match value {
                $($ty::$variant => $text,)+
            }
        }

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
        pub const $all: [Self; [$(stringify!($variant)),+].len()] = [$(Self::$variant),+];
    };
    (@parse [] $($variant:ident => $text:literal,)+) => {};
    (@parse [$parse:ident] $($variant:ident => $text:literal,)+) => {
        pub fn $parse(text: &str) -> Option<Self> {
            match text {
                $($text => Some(Self::$variant),)+
                _ => None,
            }
        }
    };
}

pub(crate) use wire_text;
