//! Tiny JSON value builder for trace records (no external dependencies).

#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Null,
    Bool(bool),
    Int(i64),
    /// Floats are written with 3 decimals (enough for positions/headings).
    Float(f32),
    Str(String),
    /// Pointer-like value, written as a "0x…" string; diff tools drop keys ending in `ptr`.
    Hex(u32),
    List(Vec<Val>),
    Obj(Vec<(String, Val)>),
}

impl Val {
    pub fn obj() -> Self {
        Val::Obj(Vec::new())
    }
    /// Appends a key (only for `Obj`).
    pub fn put(&mut self, k: &str, v: impl Into<Val>) {
        if let Val::Obj(f) = self {
            f.push((k.to_string(), v.into()));
        }
    }
    pub fn with(mut self, k: &str, v: impl Into<Val>) -> Self {
        self.put(k, v);
        self
    }
    pub fn write(&self, out: &mut String) {
        match self {
            Val::Null => out.push_str("null"),
            Val::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Val::Int(i) => out.push_str(&i.to_string()),
            Val::Float(f) if f.is_finite() => out.push_str(&format!("{f:.3}")),
            Val::Float(f) => write_str(out, &f.to_string()),
            Val::Str(s) => write_str(out, s),
            Val::Hex(h) => write_str(out, &format!("{h:#x}")),
            Val::List(l) => {
                out.push('[');
                for (i, v) in l.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Val::Obj(f) => {
                out.push('{');
                for (i, (k, v)) in f.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(out, k);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
    pub fn to_json(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }
}

pub fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl From<bool> for Val {
    fn from(v: bool) -> Self {
        Val::Bool(v)
    }
}
impl From<i32> for Val {
    fn from(v: i32) -> Self {
        Val::Int(v.into())
    }
}
impl From<u32> for Val {
    fn from(v: u32) -> Self {
        Val::Int(v.into())
    }
}
impl From<i64> for Val {
    fn from(v: i64) -> Self {
        Val::Int(v)
    }
}
impl From<u64> for Val {
    fn from(v: u64) -> Self {
        Val::Int(v as i64)
    }
}
impl From<f32> for Val {
    fn from(v: f32) -> Self {
        Val::Float(v)
    }
}
impl From<&str> for Val {
    fn from(v: &str) -> Self {
        Val::Str(v.to_string())
    }
}
impl From<String> for Val {
    fn from(v: String) -> Self {
        Val::Str(v)
    }
}
impl<T: Into<Val>> From<Option<T>> for Val {
    fn from(v: Option<T>) -> Self {
        v.map(Into::into).unwrap_or(Val::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_json() {
        let v = Val::obj()
            .with("fn", "CWorld::Add")
            .with("n", 3)
            .with("x", 1.5f32)
            .with("ptr", Val::Hex(0x1234))
            .with("l", Val::List(vec![Val::Bool(true), Val::Null]))
            .with("s", "a\"b\\\n\u{1}");
        assert_eq!(
            v.to_json(),
            r#"{"fn":"CWorld::Add","n":3,"x":1.500,"ptr":"0x1234","l":[true,null],"s":"a\"b\\\n\u0001"}"#
        );
        assert_eq!(Val::Float(f32::NAN).to_json(), "\"NaN\"");
    }
}
