//! The phase-1 exports (spec §9 scope call), weapon slots (§6.3, ruling V-4) and what they share: the files, the
//! table pattern (§6.5) and the shapes several tables use (handles, tags, tag queries, soft paths).

pub mod catalog;
pub mod hunts;
pub mod mastery;
pub mod progression;
pub mod slayers;
pub mod store;
pub mod weapons;

use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};

use crate::engine::{Engine, Num, short, sorted_rows};
use crate::json::Json;
use crate::sdk;
use crate::sdk_parse::Member;
use crate::status::Status;

/// A table with more rows than this is skipped (spec §6.5: `[0, 200000)`).
pub const ROW_LIMIT: i32 = 200_000;

pub struct Ctx<'e, 'a> {
    pub engine: &'e Engine<'a>,
    pub out: PathBuf,
    pub status: &'e Status,
}

/// A file written as the old text-mode streams wrote it: every line ends `\r\n` (spec §5).
pub struct TextFile {
    file: BufWriter<File>,
}

impl TextFile {
    /// Opens `path` truncating it.
    pub fn create(path: &Path) -> Option<Self> {
        File::create(path).ok().map(|file| Self {
            file: BufWriter::new(file),
        })
    }

    pub fn line(&mut self, json: &[u8]) {
        let _ = self.file.write_all(json);
        let _ = self.file.write_all(b"\r\n");
    }

    pub fn text(&mut self, text: &str) {
        let _ = self.file.write_all(&crate::json::crlf(text.as_bytes()));
    }

    pub fn finish(mut self) {
        let _ = self.file.flush();
    }
}

impl Json {
    pub fn num(&mut self, value: Num) -> &mut Self {
        match value {
            Num::Int(value) => self.int(value),
            Num::Float(value) => self.float(value),
        }
    }
}

/// Writes one object's fields from the members of the struct at `at`.
pub struct Row<'j, 'e, 'a> {
    pub j: &'j mut Json,
    pub e: &'e Engine<'a>,
    pub at: u64,
}

impl Row<'_, '_, '_> {
    pub fn s(&mut self, key: &str, member: &Member) -> &mut Self {
        let text = self.e.text(self.at, member);
        self.j.kstr(key, &text);
        self
    }

    pub fn n(&mut self, key: &str, member: &Member) -> &mut Self {
        let value = self.e.number(self.at, member);
        self.j.key(key).num(value);
        self
    }

    pub fn b(&mut self, key: &str, member: &Member) -> &mut Self {
        let value = self.e.flag(self.at, member);
        self.j.kbool(key, value);
        self
    }

    pub fn count(&mut self, key: &str, member: &Member) -> &mut Self {
        let value = self.e.count(self.at, member);
        self.j.kint(key, value);
        self
    }

    pub fn list(&mut self, key: &str, member: &Member) -> &mut Self {
        let values = self.e.strings(self.at, member);
        self.j.key(key).arr();
        for value in &values {
            self.j.str(value);
        }
        self.j.end_arr();
        self
    }

    pub fn nums(&mut self, key: &str, member: &Member) -> &mut Self {
        let values = self.e.numbers(self.at, member);
        self.j.key(key).arr();
        for value in values {
            self.j.num(value);
        }
        self.j.end_arr();
        self
    }

    /// `{"table":s,"rowName":s}` (spec §6.10).
    pub fn handle(&mut self, key: &str, member: &Member) -> &mut Self {
        self.j.key(key);
        handle(self.j, self.e, self.at.wrapping_add(u64::from(member.offset)));
        self
    }

    pub fn handles(&mut self, key: &str, member: &Member) -> &mut Self {
        let elements = self.e.items(self.at, member);
        self.j.key(key).arr();
        for element in elements {
            handle(self.j, self.e, element);
        }
        self.j.end_arr();
        self
    }

    /// A tag container's own tags (spec §6.10).
    pub fn tags(&mut self, key: &str, member: &Member) -> &mut Self {
        let tags = self
            .e
            .strings(self.at.wrapping_add(u64::from(member.offset)), &sdk::TAG_CONTAINER_TAGS);
        self.j.key(key).arr();
        for tag in &tags {
            self.j.str(tag);
        }
        self.j.end_arr();
        self
    }

    /// A soft pointer's path, raw form, `None` emptied (spec §4.5, hunts only).
    pub fn soft(&mut self, key: &str, member: &Member) -> &mut Self {
        let path = soft_raw(self.e, self.at.wrapping_add(u64::from(member.offset)));
        self.j.kstr(key, &path);
        self
    }

    pub fn softs(&mut self, key: &str, member: &Member) -> &mut Self {
        let paths: Vec<String> = self
            .e
            .items(self.at, member)
            .into_iter()
            .map(|at| soft_raw(self.e, at))
            .collect();
        self.j.key(key).arr();
        for path in &paths {
            self.j.str(path);
        }
        self.j.end_arr();
        self
    }
}

pub fn handle(j: &mut Json, e: &Engine, at: u64) {
    let table = e.name_of(e.ptr(at.wrapping_add(u64::from(sdk::ROW_HANDLE_TABLE.offset))));
    let row = e.fname(at.wrapping_add(u64::from(sdk::ROW_HANDLE_ROW.offset)));
    j.obj().kstr("table", &table).kstr("rowName", &row).end_obj();
}

/// `{"tokenStreamVersion":n,"tagDictionary":[s],"tokenStream":[n],"userDescription":s,"autoDescription":s}`.
pub fn tag_query(j: &mut Json, e: &Engine, at: u64) {
    j.obj();
    Row { j, e, at }
        .n("tokenStreamVersion", &sdk::TAG_QUERY_VERSION)
        .list("tagDictionary", &sdk::TAG_QUERY_DICTIONARY)
        .nums("tokenStream", &sdk::TAG_QUERY_STREAM)
        .s("userDescription", &sdk::TAG_QUERY_USER_DESCRIPTION)
        .s("autoDescription", &sdk::TAG_QUERY_AUTO_DESCRIPTION);
    j.end_obj();
}

pub fn soft_raw(e: &Engine, at: u64) -> String {
    let path = e.soft_path_raw(at);
    if path == "None" { String::new() } else { path }
}

/// A member's offset inside the struct `outer` names (both offsets from the same row).
pub const fn within(member: Member, outer: &Member) -> Member {
    Member {
        offset: member.offset.saturating_sub(outer.offset),
        ..member
    }
}

/// How an export reacts to a table's rows.
pub struct TableExport<'p> {
    pub prefix: &'p str,
    pub row_struct: &'p str,
    pub file: PathBuf,
    /// Whether the row limit is inclusive (`[0, 200000]`, store images) or not (§6.5).
    pub inclusive: bool,
    /// The status line when no table has the row struct.
    pub missing: String,
    /// A subfolder of the output made before the file is opened.
    pub folder: Option<PathBuf>,
}

/// Pattern §6.5: every table with the row struct, rows sorted, each serialized by `row`; a row whose memory faults
/// is skipped (spec §12 call 4). 0 and no file when there is no table; -1 when the file cannot be opened.
pub fn export_tables(ctx: &Ctx, spec: &TableExport, mut row: impl FnMut(&mut Json, &Engine, &str, &str, u64)) -> i64 {
    let e = ctx.engine;
    let tables = e.tables_by_row_struct(spec.row_struct);
    if tables.is_empty() {
        ctx.status.line(spec.prefix, &spec.missing);
        return 0;
    }
    if let Some(folder) = &spec.folder {
        let _ = std::fs::create_dir(folder);
    }
    let Some(mut file) = TextFile::create(&spec.file) else {
        ctx.status
            .line(spec.prefix, &format!("cannot open {}", spec.file.display()));
        return -1;
    };
    let name = spec
        .file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut written = 0i64;
    for table in tables {
        let table_name = e.name_of(table);
        let (count, rows) = e.rows(table);
        let limit_ok = if spec.inclusive {
            count <= ROW_LIMIT
        } else {
            count < ROW_LIMIT
        };
        if count < 0 || !limit_ok {
            ctx.status.line(
                spec.prefix,
                &format!("{table_name}: row count {count} out of range, skipping"),
            );
            continue;
        }
        let mut table_written = 0;
        for (row_name, data) in sorted_rows(&rows) {
            e.take_fault();
            let mut j = Json::new();
            if data != 0 {
                row(&mut j, e, &table_name, &row_name, data);
            }
            if data == 0 || e.take_fault() {
                ctx.status
                    .line(spec.prefix, &format!("{table_name}/{row_name}: unreadable row skipped"));
                continue;
            }
            file.line(&j.into_bytes());
            table_written += 1;
        }
        ctx.status.line(
            spec.prefix,
            &format!(
                "wrote {table_written}/{count} {} rows -> {name} from {table_name}",
                spec.row_struct
            ),
        );
        written += table_written;
    }
    file.finish();
    written
}

/// The live object's short class name; used for `objectClass`.
pub fn class_name(e: &Engine, object: u64) -> String {
    short(&e.raw_name_of(e.class_of(object))).to_owned()
}
