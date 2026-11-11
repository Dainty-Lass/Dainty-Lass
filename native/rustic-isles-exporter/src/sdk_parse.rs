//! Reads the Dumper-7 headers a host generated from their own client, for the members, enum values, sizes and
//! constants `facts.rs` names (spec §9). Shared by `build.rs`, the library and the fuzz target; nothing from an SDK is
//! committed (AGENTS.md), so the tests run on `fixtures/synthetic-sdk`, which is made up.

use std::collections::HashMap;

/// How a member is read and written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool,
    /// A `uint8` bitfield member: the bit's index.
    Bit(u8),
    Int {
        size: u8,
        signed: bool,
    },
    Float {
        size: u8,
    },
    Str,
    Name,
    Text,
    ObjPtr,
    ClassPtr,
    SoftPtr,
    Array,
    Map,
    Tag,
    TagContainer,
    TagQuery,
    RowHandle,
    DateTime,
    Struct,
    Other,
}

/// Where a member sits and what it holds. Offsets are from the start of the struct the path ends in, or from the
/// array element after the path's last `[]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    pub offset: u32,
    pub size: u32,
    pub kind: Kind,
    /// Arrays: the element; maps: the key.
    pub elem: Kind,
    /// Arrays: the element size; maps: the size of a set element (pair and hash links).
    pub stride: u32,
    /// Maps: the value, and the key's and value's offsets in an element.
    pub value: Kind,
    pub key_offset: u32,
    pub value_offset: u32,
}

impl Member {
    pub const fn plain(offset: u32, size: u32, kind: Kind) -> Self {
        Self {
            offset,
            size,
            kind,
            elem: Kind::Other,
            stride: 0,
            value: Kind::Other,
            key_offset: 0,
            value_offset: 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Field {
    pub name: String,
    pub ty: String,
    pub offset: u32,
    pub size: u32,
    pub bit: Option<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct StructDef {
    pub parent: Option<String>,
    pub size: Option<u32>,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, Default)]
pub struct EnumDef {
    pub underlying: String,
    pub values: Vec<(String, i64)>,
}

#[derive(Debug, Default)]
pub struct Sdk {
    pub structs: HashMap<String, StructDef>,
    pub enums: HashMap<String, EnumDef>,
    pub consts: HashMap<String, i64>,
}

enum Frame {
    Namespace(String),
    Struct(String),
    Enum(Option<String>),
    Other,
}

enum Pending {
    None,
    Namespace(String),
    Struct(String, StructDef),
    Enum(Option<String>, String),
}

/// Longest line and depth considered; Dumper-7 stays far below both.
const MAX_LINE: usize = 4096;
const MAX_DEPTH: usize = 64;
/// Path segments and parent links followed before giving up (a loop in a malformed SDK).
const MAX_HOPS: usize = 64;

impl Sdk {
    /// Adds one header's declarations. Never fails: anything it does not recognise is skipped, and a fact that
    /// needed it fails in `resolve` instead, naming what is missing.
    pub fn parse(&mut self, text: &str) {
        let mut stack: Vec<Frame> = Vec::new();
        let mut pending = Pending::None;
        let mut size_comment: Option<u32> = None;
        for raw in text.lines() {
            if raw.len() > MAX_LINE {
                continue;
            }
            let line = raw.trim();
            if let Some(comment) = line.strip_prefix("//") {
                if let Some(size) = size_from_comment(comment) {
                    size_comment = Some(size);
                }
                continue;
            }
            let (code, comment) = match line.find("//") {
                Some(at) => (line[..at].trim(), Some(&line[at + 2..])),
                None => (line, None),
            };
            if code.is_empty() {
                continue;
            }
            let mut header = false;
            if let Some(found) = header_of(code) {
                pending = match found {
                    Header::Namespace(name) => Pending::Namespace(name),
                    Header::Struct(name, parent) => Pending::Struct(
                        name,
                        StructDef {
                            parent,
                            size: size_comment.take(),
                            fields: Vec::new(),
                        },
                    ),
                    Header::Enum(name, underlying) => Pending::Enum(name, underlying),
                };
                header = true;
            } else {
                self.member_or_value(code, comment, &mut stack);
            }
            let sole_brace = code.starts_with('{');
            let mut first = true;
            for ch in code.chars() {
                match ch {
                    '{' => {
                        let frame = if first && (header || sole_brace) {
                            match std::mem::replace(&mut pending, Pending::None) {
                                Pending::Namespace(name) => Frame::Namespace(name),
                                Pending::Struct(name, def) => {
                                    self.structs.entry(name.clone()).or_insert(def);
                                    Frame::Struct(name)
                                }
                                Pending::Enum(name, underlying) => {
                                    if let Some(name) = &name {
                                        self.enums.entry(name.clone()).or_insert(EnumDef {
                                            underlying,
                                            values: Vec::new(),
                                        });
                                    }
                                    Frame::Enum(name)
                                }
                                Pending::None => Frame::Other,
                            }
                        } else {
                            Frame::Other
                        };
                        first = false;
                        if stack.len() < MAX_DEPTH {
                            stack.push(frame);
                        }
                    }
                    '}' => {
                        stack.pop();
                    }
                    _ => {}
                }
            }
            if code.ends_with(';') && !header {
                pending = Pending::None;
            }
        }
    }

    fn member_or_value(&mut self, code: &str, comment: Option<&str>, stack: &mut [Frame]) {
        if let Some((name, value)) = constexpr(code) {
            let scope = stack.iter().rev().find_map(|frame| match frame {
                Frame::Namespace(name) | Frame::Struct(name) => Some(name.as_str()),
                _ => None,
            });
            if let Some(scope) = scope {
                self.consts.entry(format!("{scope}::{name}")).or_insert(value);
            }
            self.consts.entry(name).or_insert(value);
            return;
        }
        match stack.last() {
            Some(Frame::Enum(name)) => {
                let Some((enumerator, value)) = enumerator(code) else {
                    return;
                };
                match name {
                    Some(name) => {
                        if let Some(def) = self.enums.get_mut(name) {
                            def.values.push((enumerator, value));
                        }
                    }
                    None => {
                        let scope = stack.iter().rev().find_map(|frame| match frame {
                            Frame::Struct(name) => Some(name.clone()),
                            _ => None,
                        });
                        if let Some(scope) = scope {
                            self.consts.entry(format!("{scope}::{enumerator}")).or_insert(value);
                        }
                        self.consts.entry(enumerator).or_insert(value);
                    }
                }
            }
            Some(Frame::Struct(name)) => {
                let Some(field) = comment.and_then(|comment| field(code, comment)) else {
                    return;
                };
                if let Some(def) = self.structs.get_mut(name) {
                    def.fields.push(field);
                }
            }
            _ => {}
        }
    }

    fn find_field(&self, owner: &str, name: &str) -> Result<&Field, String> {
        let mut current = owner;
        for _ in 0..MAX_HOPS {
            let def = self
                .structs
                .get(current)
                .ok_or_else(|| format!("struct {current} is not in the SDK"))?;
            let found = match name.strip_prefix('@') {
                Some(pattern) => def.fields.iter().find(|field| field.ty.contains(pattern)),
                None => def.fields.iter().find(|field| field.name == name),
            };
            if let Some(field) = found {
                return Ok(field);
            }
            match &def.parent {
                Some(parent) => current = parent,
                None => break,
            }
        }
        Err(format!("no member {name} in {owner} or its parents"))
    }

    /// `Struct.Member`, `Struct.Array[].Member`, `Struct.Embedded.Member`; `@text` matches a member by its type.
    pub fn resolve(&self, path: &str) -> Result<Member, String> {
        let mut segments = path.split('.');
        let mut owner = segments
            .next()
            .filter(|s| !s.is_empty())
            .ok_or("empty path")?
            .to_owned();
        let segments: Vec<&str> = segments.collect();
        if segments.is_empty() || segments.len() > MAX_HOPS {
            return Err(format!("{path}: needs Struct.Member"));
        }
        let mut base = 0u32;
        for (index, segment) in segments.iter().enumerate() {
            let last = index + 1 == segments.len();
            let (name, element) = match segment.strip_suffix("[]") {
                Some(name) => (name, true),
                None => (*segment, false),
            };
            let field = self.find_field(&owner, name).map_err(|e| format!("{path}: {e}"))?;
            let offset = base.saturating_add(field.offset);
            if element {
                let inner = template_args(&field.ty, "TArray")
                    .and_then(|args| args.first().map(|arg| bare_type(arg)))
                    .ok_or_else(|| format!("{path}: {name} is not a TArray ({})", field.ty))?;
                if last {
                    return Err(format!("{path}: ends in []"));
                }
                if !self.structs.contains_key(&inner) {
                    return Err(format!("{path}: element type {inner} of {name} is not in the SDK"));
                }
                owner = inner;
                base = 0;
            } else if last {
                return self.member_of(field, offset).map_err(|e| format!("{path}: {e}"));
            } else {
                let inner = bare_type(&field.ty);
                if !self.structs.contains_key(&inner) {
                    return Err(format!("{path}: {name} is not a struct in the SDK ({})", field.ty));
                }
                owner = inner;
                base = offset;
            }
        }
        Err(format!("{path}: unresolved"))
    }

    fn member_of(&self, field: &Field, offset: u32) -> Result<Member, String> {
        let kind = self.kind_of(&field.ty, field.size, field.bit);
        let mut member = Member::plain(offset, field.size, kind);
        if let Some(args) = template_args(&field.ty, "TArray") {
            let arg = args.first().ok_or("TArray without an argument")?;
            member.kind = Kind::Array;
            member.elem = self.kind_of(arg, 0, None);
            member.stride = self.size_of(arg)?;
        } else if let Some(args) = template_args(&field.ty, "TMap") {
            let [key, value] = args.as_slice() else {
                return Err(format!("TMap needs two arguments ({})", field.ty));
            };
            member.kind = Kind::Map;
            let (key_size, key_align) = (self.size_of(key)?, self.align_of(key));
            let (value_size, value_align) = (self.size_of(value)?, self.align_of(value));
            let pair_align = key_align.max(value_align).max(4);
            member.value_offset = align(key_size, value_align);
            let pair = align(member.value_offset + value_size, pair_align);
            // TSetElement: the pair, then HashNextId and HashIndex (two int32).
            member.stride = align(align(pair, 4) + 8, pair_align);
            member.elem = self.kind_of(key, 0, None);
            member.value = self.kind_of(value, 0, None);
            if member.elem == Kind::Struct {
                // A struct key (a gameplay attribute) is named by its first member when that is a name or string.
                if let Some(first) = self.structs.get(&bare_type(key)).and_then(|def| def.fields.first()) {
                    let first_kind = self.kind_of(&first.ty, first.size, first.bit);
                    if matches!(first_kind, Kind::Str | Kind::Name) {
                        member.elem = first_kind;
                        member.key_offset = first.offset;
                    }
                }
            }
        }
        Ok(member)
    }

    fn kind_of(&self, ty: &str, size: u32, bit: Option<u8>) -> Kind {
        let ty = ty.trim();
        if ty.ends_with('*') {
            return if bare_type(ty.trim_end_matches('*')) == "UClass" {
                Kind::ClassPtr
            } else {
                Kind::ObjPtr
            };
        }
        if template_args(ty, "TSubclassOf").is_some() {
            return Kind::ClassPtr;
        }
        if template_args(ty, "TSoftObjectPtr").is_some() || template_args(ty, "TSoftClassPtr").is_some() {
            return Kind::SoftPtr;
        }
        if template_args(ty, "TArray").is_some() {
            return Kind::Array;
        }
        if template_args(ty, "TMap").is_some() {
            return Kind::Map;
        }
        let bare = bare_type(ty);
        match bare.as_str() {
            "bool" => match bit {
                Some(bit) => Kind::Bit(bit),
                None => Kind::Bool,
            },
            "uint8" => match bit {
                Some(bit) => Kind::Bit(bit),
                None => Kind::Int { size: 1, signed: false },
            },
            "int8" => Kind::Int { size: 1, signed: true },
            "uint16" => Kind::Int { size: 2, signed: false },
            "int16" => Kind::Int { size: 2, signed: true },
            "uint32" => Kind::Int { size: 4, signed: false },
            "int32" => Kind::Int { size: 4, signed: true },
            "uint64" => Kind::Int { size: 8, signed: false },
            "int64" => Kind::Int { size: 8, signed: true },
            "float" => Kind::Float { size: 4 },
            "double" => Kind::Float { size: 8 },
            "FString" => Kind::Str,
            "FName" => Kind::Name,
            "FText" => Kind::Text,
            "FGameplayTag" => Kind::Tag,
            "FGameplayTagContainer" => Kind::TagContainer,
            "FGameplayTagQuery" => Kind::TagQuery,
            "FDataTableRowHandle" => Kind::RowHandle,
            "FDateTime" => Kind::DateTime,
            _ => {
                if let Some(def) = self.enums.get(&bare) {
                    let size = primitive_size(&def.underlying).unwrap_or(if size == 0 { 1 } else { size });
                    Kind::Int {
                        size: u8::try_from(size).unwrap_or(1),
                        signed: def.underlying.starts_with("int"),
                    }
                } else if self.structs.contains_key(&bare) {
                    Kind::Struct
                } else {
                    Kind::Other
                }
            }
        }
    }

    fn size_of(&self, ty: &str) -> Result<u32, String> {
        let ty = ty.trim();
        if ty.ends_with('*') || template_args(ty, "TSubclassOf").is_some() {
            return Ok(8);
        }
        if template_args(ty, "TArray").is_some() {
            return Ok(16);
        }
        if template_args(ty, "TMap").is_some() {
            return Ok(0x50);
        }
        if template_args(ty, "TSoftObjectPtr").is_some() || template_args(ty, "TSoftClassPtr").is_some() {
            return Ok(0x28);
        }
        let bare = bare_type(ty);
        if let Some(size) = primitive_size(&bare) {
            return Ok(size);
        }
        match bare.as_str() {
            "FString" => return Ok(16),
            "FName" => return Ok(8),
            "FText" => return Ok(0x18),
            _ => {}
        }
        if let Some(def) = self.enums.get(&bare) {
            return Ok(primitive_size(&def.underlying).unwrap_or(1));
        }
        self.structs
            .get(&bare)
            .and_then(|def| def.size)
            .ok_or_else(|| format!("no size for {bare}"))
    }

    fn align_of(&self, ty: &str) -> u32 {
        let bare = bare_type(ty);
        if let Some(size) = primitive_size(&bare) {
            return size;
        }
        if bare == "FName" {
            return 4;
        }
        if let Some(def) = self.enums.get(&bare) {
            return primitive_size(&def.underlying).unwrap_or(1);
        }
        if let Some(size) = self.structs.get(&bare).and_then(|def| def.size)
            && size % 8 != 0
        {
            return 4;
        }
        8
    }

    pub fn size(&self, name: &str) -> Result<u32, String> {
        self.structs
            .get(name)
            .and_then(|def| def.size)
            .ok_or_else(|| format!("size of {name}: not in the SDK"))
    }

    pub fn constant(&self, name: &str) -> Result<i64, String> {
        self.consts
            .get(name)
            .copied()
            .ok_or_else(|| format!("constant {name}: not in the SDK"))
    }

    /// `EName::Enumerator`, or `Struct.Member::Enumerator` for the enum a member is typed with. An enumerator matches
    /// by its whole name or a `_`-separated last part (`EMasteryCardType_Weapon` for `Weapon`).
    pub fn enum_value(&self, path: &str) -> Result<i64, String> {
        let (left, enumerator) = path.rsplit_once("::").ok_or_else(|| format!("{path}: needs ::"))?;
        let enum_name = if left.contains('.') {
            let (owner, member) = left.split_once('.').ok_or_else(|| format!("{path}: bad path"))?;
            let field = self.find_field(owner, member).map_err(|e| format!("{path}: {e}"))?;
            bare_type(&field.ty)
        } else {
            left.to_owned()
        };
        let def = self
            .enums
            .get(&enum_name)
            .ok_or_else(|| format!("{path}: enum {enum_name} is not in the SDK"))?;
        let suffix = format!("_{enumerator}");
        def.values
            .iter()
            .find(|(name, _)| name == enumerator)
            .or_else(|| def.values.iter().find(|(name, _)| name.ends_with(&suffix)))
            .map(|(_, value)| *value)
            .ok_or_else(|| format!("{path}: no enumerator {enumerator} in {enum_name}"))
    }
}

pub const fn align(value: u32, to: u32) -> u32 {
    if to == 0 { value } else { value.div_ceil(to) * to }
}

fn primitive_size(ty: &str) -> Option<u32> {
    Some(match ty {
        "bool" | "uint8" | "int8" | "char" => 1,
        "uint16" | "int16" | "wchar_t" => 2,
        "uint32" | "int32" | "float" => 4,
        "uint64" | "int64" | "double" => 8,
        _ => return None,
    })
}

/// The type without `class `/`struct `/`enum class `/`const`, spaces or pointer stars.
pub fn bare_type(ty: &str) -> String {
    ty.split_whitespace()
        .filter(|word| !matches!(*word, "class" | "struct" | "enum" | "const" | "union"))
        .collect::<Vec<_>>()
        .join("")
        .trim_end_matches('*')
        .to_owned()
}

/// The top-level arguments of `Name<...>`, or `None` when `ty` is not that template.
fn template_args(ty: &str, name: &str) -> Option<Vec<String>> {
    let ty = ty.trim();
    let rest = ty.strip_prefix(name)?.trim_start();
    let inner = rest.strip_prefix('<')?.trim_end().strip_suffix('>')?;
    let mut args = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (at, ch) in inner.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.checked_sub(1)?,
            ',' if depth == 0 => {
                args.push(inner[start..at].trim().to_owned());
                start = at + 1;
            }
            _ => {}
        }
    }
    args.push(inner[start..].trim().to_owned());
    Some(args)
}

enum Header {
    Namespace(String),
    Struct(String, Option<String>),
    Enum(Option<String>, String),
}

fn header_of(code: &str) -> Option<Header> {
    let code = code.trim_end_matches('{').trim();
    if code.ends_with(';') || code.contains('=') {
        return None;
    }
    let mut words = code.split_whitespace();
    match words.next()? {
        "namespace" => Some(Header::Namespace(words.next()?.to_owned())),
        "class" | "struct" | "union" => {
            if code.contains("(") && !code.contains("alignas(") {
                return None;
            }
            let rest: Vec<&str> = words.filter(|word| !word.starts_with("alignas(")).collect();
            let name = rest.first()?.trim_end_matches(':');
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || name.is_empty() {
                return None;
            }
            let parent = rest
                .iter()
                .position(|word| *word == "public")
                .and_then(|at| rest.get(at + 1))
                .map(|parent| parent.trim_end_matches(',').to_owned());
            Some(Header::Struct(name.to_owned(), parent))
        }
        "enum" => {
            let rest: Vec<&str> = words.filter(|word| *word != "class").collect();
            let (name, underlying) = match rest.as_slice() {
                [] => (None, "int32".to_owned()),
                [":", underlying, ..] => (None, (*underlying).to_owned()),
                [name] => (Some((*name).to_owned()), "int32".to_owned()),
                [name, ":", underlying, ..] => (Some((*name).to_owned()), (*underlying).to_owned()),
                _ => return None,
            };
            Some(Header::Enum(name, underlying))
        }
        _ => None,
    }
}

/// `// 0x0010 (0x0040 - 0x0030)`: the struct's total size is the second number.
fn size_from_comment(comment: &str) -> Option<u32> {
    let comment = comment.trim();
    let (_, rest) = comment.split_once('(')?;
    let (total, _) = rest.split_once('-')?;
    let first = comment.split_whitespace().next()?;
    parse_int(first)?;
    u32::try_from(parse_int(total.trim())?).ok()
}

/// `constexpr int32 Name = 0x10;` (with `static`/`inline` in front).
fn constexpr(code: &str) -> Option<(String, i64)> {
    let at = code.find("constexpr ")?;
    let (left, right) = code[at..].split_once('=')?;
    let name = left.split_whitespace().last()?;
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let value = parse_int(right.trim().trim_end_matches(';').trim())?;
    Some((name.to_owned(), value))
}

/// `Name = 3,` in an enum body.
fn enumerator(code: &str) -> Option<(String, i64)> {
    let (name, value) = code.split_once('=')?;
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((name.to_owned(), parse_int(value.trim().trim_end_matches(',').trim())?))
}

/// A member line: `<type> <name>[ : bits][[n]];` with `0xOFFSET(0xSIZE)` in its comment.
fn field(code: &str, comment: &str) -> Option<Field> {
    let decl = code.strip_suffix(';')?.trim();
    if decl.contains('(') || decl.starts_with("static ") || decl.starts_with("using ") {
        return None;
    }
    let comment = comment.trim();
    let (offset, rest) = comment.split_once('(')?;
    let offset = u32::try_from(parse_int(offset.trim())?).ok()?;
    let (size, flags) = rest.split_once(')')?;
    let size = u32::try_from(parse_int(size.trim())?).ok()?;
    let bit = flags
        .find("BitIndex:")
        .and_then(|at| flags[at + 9..].split([',', ')']).next())
        .and_then(|text| parse_int(text.trim()))
        .and_then(|bit| u8::try_from(bit).ok());
    let decl = match decl.split_once(':') {
        Some((left, right)) if right.trim().chars().all(|c| c.is_ascii_digit()) => left.trim(),
        _ => decl,
    };
    let decl = match decl.find('[') {
        Some(at) if decl.ends_with(']') => decl[..at].trim(),
        _ => decl,
    };
    let split = decl.rfind(|c: char| c.is_whitespace() || c == '*' || c == '>')?;
    let name = decl[split + 1..].trim();
    let ty = decl[..=split].trim();
    if name.is_empty() || ty.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some(Field {
        name: name.to_owned(),
        ty: ty.split_whitespace().collect::<Vec<_>>().join(" "),
        offset,
        size,
        bit,
    })
}

/// Decimal or `0x` hex, optionally negative, with C suffixes (`u`, `ULL`) dropped.
pub fn parse_int(text: &str) -> Option<i64> {
    let text = text.trim();
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest.trim()),
        None => (false, text),
    };
    let text = text.trim_end_matches(['u', 'U', 'l', 'L']);
    let value = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16).ok()?,
        None => text.parse::<i64>().ok()?,
    };
    Some(if negative { value.checked_neg()? } else { value })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
namespace Offsets
{
	constexpr int32 GObjects          = 0x00001000;
	constexpr int32 ProcessEventIdx   = 0x00000044;
}

// Enum Test.EKind
// NumValues: 0x0003
enum class EKind : uint8
{
	EKind_First                              = 0,
	EKind_Second                             = 7,
	EKind_MAX                                = 8,
};

// ScriptStruct Test.Inner
// 0x0010 (0x0010 - 0x0000)
struct FInner final
{
public:
	class FName                                   Id;                                                // 0x0000(0x0008)(Edit)
	int32                                         Amount;                                            // 0x0008(0x0004)(Edit)
	uint8                                         Pad_C[0x4];                                        // 0x000C(0x0004)(Fixing Struct Size After Last Property [ Dumper-7 ])
};

// ScriptStruct Test.Base
// 0x0020 (0x0020 - 0x0000)
struct FBase
{
public:
	class FString                                 Label;                                             // 0x0000(0x0010)(Edit)
	struct FInner                                 One;                                               // 0x0010(0x0010)(Edit)
};

// ScriptStruct Test.Row
// 0x0080 (0x00A0 - 0x0020)
struct alignas(0x08) FRow final : public FBase
{
public:
	TArray<struct FInner>                         List;                                              // 0x0020(0x0010)(Edit)
	uint8                                         bFlag : 1;                                         // 0x0030(0x0001)(BitIndex: 0x02, PropSize: 0x0001 (Edit))
	EKind                                         Kind;                                              // 0x0031(0x0001)(Edit)
	TMap<class FName, float>                      Values;                                            // 0x0038(0x0050)(Edit)
	class UObject*                                Thing;                                             // 0x0088(0x0008)(Edit)
	TSoftObjectPtr<class UObject>                 Asset;                                             // 0x0090(0x0028)(Edit)
};
"#;

    fn sample() -> Sdk {
        let mut sdk = Sdk::default();
        sdk.parse(SAMPLE);
        sdk
    }

    #[test]
    fn reads_members_parents_arrays_and_maps() {
        let sdk = sample();
        assert_eq!(sdk.resolve("FRow.Label").unwrap(), Member::plain(0, 16, Kind::Str));
        assert_eq!(
            sdk.resolve("FRow.One.Amount").unwrap(),
            Member::plain(0x18, 4, Kind::Int { size: 4, signed: true })
        );
        let list = sdk.resolve("FRow.List").unwrap();
        assert_eq!((list.kind, list.elem, list.stride), (Kind::Array, Kind::Struct, 0x10));
        assert_eq!(sdk.resolve("FRow.List[].Amount").unwrap().offset, 8);
        assert_eq!(sdk.resolve("FRow.bFlag").unwrap().kind, Kind::Bit(2));
        assert_eq!(
            sdk.resolve("FRow.Kind").unwrap().kind,
            Kind::Int { size: 1, signed: false }
        );
        let map = sdk.resolve("FRow.Values").unwrap();
        assert_eq!(
            (map.elem, map.value, map.value_offset, map.stride),
            (Kind::Name, Kind::Float { size: 4 }, 8, 0x14)
        );
        assert_eq!(sdk.resolve("FRow.Thing").unwrap().kind, Kind::ObjPtr);
        assert_eq!(sdk.resolve("FRow.Asset").unwrap().kind, Kind::SoftPtr);
        assert_eq!(sdk.resolve("FRow.@TArray").unwrap().offset, 0x20);
        assert_eq!(sdk.size("FRow").unwrap(), 0xA0);
    }

    #[test]
    fn reads_constants_and_enums() {
        let sdk = sample();
        assert_eq!(sdk.constant("Offsets::GObjects").unwrap(), 0x1000);
        assert_eq!(sdk.constant("ProcessEventIdx").unwrap(), 0x44);
        assert_eq!(sdk.enum_value("EKind::EKind_Second").unwrap(), 7);
        assert_eq!(sdk.enum_value("FRow.Kind::Second").unwrap(), 7);
    }

    #[test]
    fn names_what_is_missing() {
        let sdk = sample();
        let error = sdk.resolve("FRow.Nope").unwrap_err();
        assert!(error.contains("FRow.Nope") && error.contains("Nope"), "{error}");
        assert!(sdk.resolve("FMissing.X").unwrap_err().contains("FMissing"));
        assert!(sdk.enum_value("FRow.Kind::Third").is_err());
        assert!(sdk.constant("Offsets::GNames").is_err());
    }
}
