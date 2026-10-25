//! A deliberately finite SceneScript arithmetic subset; never evaluates JavaScript.
//! Compilation admits only one update(value) function and lowers it to bounded,
//! straight-line numeric instructions. No host I/O, dynamic allocation, callbacks,
//! loops, recursion, dynamic code or property writes exist in the evaluator.

use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SOURCE_BYTES: usize = 8192;
pub const MAX_INSTRUCTIONS: usize = 256;
pub const MAX_BINDINGS: usize = 64;
pub const MAX_SCENE_SOURCE_BYTES: usize = 65536;
const REGISTERS: usize = 19;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScriptProgram {
    steps: Vec<(usize, Vec<Op>)>,
    result: [Vec<Op>; 3],
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
enum Op {
    Number(f64),
    Register(usize),
    Time,
    Negate,
    Binary(char),
    Function(String),
}

impl ScriptProgram {
    pub fn compile(
        source: &str,
        vector: bool,
        constants: &BTreeMap<String, f64>,
    ) -> Result<Self, String> {
        let tokens = tokenize(source)?;
        let mut parser = Parser {
            tokens,
            position: 0,
            locals: BTreeMap::new(),
            immutable: BTreeSet::new(),
            assigned: BTreeSet::new(),
            vector,
            constants,
            depth: 0,
        };
        if parser.peek() == Some("'use strict'") || parser.peek() == Some("\"use strict\"") {
            parser.position += 1;
            parser.take(";")?;
        }
        for expected in ["export", "function", "update", "(", "value", ")", "{"] {
            parser.take(expected)?;
        }
        let mut steps = Vec::new();
        while parser.peek() != Some("return") {
            if steps.len() >= 32 {
                return Err("statement budget exceeded".into());
            }
            let local = matches!(parser.peek(), Some("let" | "const"));
            let immutable = parser.peek() == Some("const");
            if local {
                parser.position += 1;
            }
            let name = parser.next()?.to_owned();
            let target = if local {
                if parser.locals.len() >= 16
                    || !identifier(&name)
                    || matches!(
                        name.as_str(),
                        "value" | "engine" | "Math" | "thisLayer" | "thisScene"
                    )
                    || parser.locals.contains_key(&name)
                    || matches!(
                        name.as_str(),
                        "return"
                            | "let"
                            | "const"
                            | "new"
                            | "true"
                            | "false"
                            | "null"
                            | "undefined"
                            | "NaN"
                            | "Infinity"
                            | "function"
                            | "export"
                            | "if"
                            | "else"
                            | "while"
                            | "for"
                            | "throw"
                            | "class"
                            | "var"
                            | "switch"
                            | "break"
                            | "continue"
                            | "try"
                            | "catch"
                            | "finally"
                            | "delete"
                            | "typeof"
                            | "void"
                            | "in"
                            | "instanceof"
                            | "do"
                            | "case"
                            | "default"
                            | "with"
                            | "debugger"
                            | "super"
                            | "this"
                            | "yield"
                            | "await"
                            | "extends"
                            | "import"
                            | "implements"
                            | "interface"
                            | "package"
                            | "private"
                            | "protected"
                            | "public"
                            | "static"
                            | "arguments"
                            | "eval"
                    )
                {
                    return Err(format!("unsupported local {name}"));
                }
                3 + parser.locals.len()
            } else {
                if parser.immutable.contains(&name) {
                    return Err(format!("cannot assign constant {name}"));
                }
                parser
                    .register(&name)
                    .ok_or_else(|| format!("unsupported write {name}"))?
            };
            parser.take("=")?;
            let code = parser.expression(0)?;
            parser.take(";")?;
            if local {
                if immutable {
                    parser.immutable.insert(name.clone());
                }
                parser.locals.insert(name, target);
            }
            steps.push((target, code));
            parser.assigned.insert(target);
        }
        parser.take("return")?;
        let result = if parser.peek() == Some("value")
            && parser
                .tokens
                .get(parser.position + 1)
                .is_some_and(|t| t == ";")
        {
            parser.position += 1;
            std::array::from_fn(|axis| vec![Op::Register(if vector { axis } else { 0 })])
        } else if vector && parser.peek() == Some("new") {
            parser.take("new")?;
            parser.take("Vec3")?;
            parser.take("(")?;
            let x = parser.expression(0)?;
            parser.take(",")?;
            let y = parser.expression(0)?;
            parser.take(",")?;
            let z = parser.expression(0)?;
            parser.take(")")?;
            [x, y, z]
        } else {
            let code = parser.expression(0)?;
            [code.clone(), code.clone(), code]
        };
        parser.take(";")?;
        parser.take("}")?;
        if parser.peek().is_some() {
            return Err("only one update(value) export is supported".into());
        }
        let program = Self { steps, result };
        if program.instruction_count() > MAX_INSTRUCTIONS {
            return Err("instruction budget exceeded".into());
        }
        Ok(program)
    }

    fn instruction_count(&self) -> usize {
        self.steps
            .iter()
            .map(|(_, code)| code.len())
            .chain(self.result.iter().map(Vec::len))
            .sum()
    }

    /// Fixed stack/register storage and at most 256 operations, including all
    /// statements/results. Results are finite, magnitude <= 1e6.
    pub fn evaluate(&self, value: [f32; 3], time: f64) -> Result<[f32; 3], String> {
        if self.steps.len() > 32
            || self.instruction_count() > MAX_INSTRUCTIONS
            || !time.is_finite()
            || time < 0.0
        {
            return Err("script execution budget or clock invalid".into());
        }
        let mut registers = [0.; REGISTERS];
        for (register, value) in registers.iter_mut().zip(value) {
            *register = f64::from(value);
        }
        let eval = |code: &[Op], registers: &[f64; REGISTERS]| -> Result<f64, String> {
            let mut stack = [0.; MAX_INSTRUCTIONS];
            let mut top = 0;
            for op in code {
                let n = match op {
                    Op::Number(n) => *n,
                    Op::Time => time,
                    Op::Register(index) => *registers.get(*index).ok_or("invalid register")?,
                    Op::Negate => {
                        if top < 1 {
                            return Err("invalid stack".into());
                        }
                        top -= 1;
                        -stack[top]
                    }
                    Op::Binary(operator) => {
                        if top < 2 {
                            return Err("invalid stack".into());
                        }
                        top -= 2;
                        let (a, b) = (stack[top], stack[top + 1]);
                        match operator {
                            '+' => a + b,
                            '-' => a - b,
                            '*' => a * b,
                            '/' => a / b,
                            '%' => a % b,
                            _ => return Err("invalid operator".into()),
                        }
                    }
                    Op::Function(name) => {
                        let arity = if matches!(name.as_str(), "Math.min" | "Math.max") {
                            2
                        } else {
                            1
                        };
                        if top < arity {
                            return Err("invalid stack".into());
                        }
                        top -= arity;
                        let a: f64 = stack[top];
                        match name.as_str() {
                            "Math.sin" => a.sin(),
                            "Math.cos" => a.cos(),
                            "Math.abs" => a.abs(),
                            "Math.floor" => a.floor(),
                            "Math.ceil" => a.ceil(),
                            "Math.round" => (a + 0.5).floor(),
                            "Math.min" => a.min(stack[top + 1]),
                            "Math.max" => a.max(stack[top + 1]),
                            _ => return Err("unsupported function".into()),
                        }
                    }
                };
                if !n.is_finite() || top >= stack.len() {
                    return Err("non-finite arithmetic or invalid stack".into());
                }
                stack[top] = n;
                top += 1;
            }
            if top != 1 {
                return Err("invalid expression stack".into());
            }
            Ok(stack[0])
        };
        for (register, code) in &self.steps {
            let n = eval(code, &registers)?;
            *registers
                .get_mut(*register)
                .ok_or("invalid write register")? = n;
        }
        let mut result = [0.; 3];
        for (out, code) in result.iter_mut().zip(&self.result) {
            let n = eval(code, &registers)?;
            if n.abs() > 1_000_000. {
                return Err("script result exceeds numeric budget".into());
            }
            *out = n as f32;
        }
        Ok(result)
    }
}

fn identifier(s: &str) -> bool {
    s.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn tokenize(source: &str) -> Result<Vec<String>, String> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err("source byte budget exceeded".into());
    }
    let bytes = source.as_bytes();
    let mut position = 0;
    let mut tokens = Vec::new();
    while position < bytes.len() {
        let b = bytes[position];
        if b.is_ascii_whitespace() {
            if matches!(b, b'\n' | b'\r') && tokens.last().is_some_and(|s| s == "return") {
                return Err("return line breaks are unsupported".into());
            }
            position += 1;
            continue;
        }
        if bytes[position..].starts_with(b"//") {
            if tokens.last().is_some_and(|s| s == "return") {
                return Err("return line comments are unsupported".into());
            }
            while position < bytes.len() && bytes[position] != b'\n' {
                position += 1;
            }
            continue;
        }
        if bytes[position..].starts_with(b"/*") {
            let start_comment = position;
            position += 2;
            while position + 1 < bytes.len() && &bytes[position..position + 2] != b"*/" {
                position += 1;
            }
            if position + 1 >= bytes.len() {
                return Err("unterminated comment".into());
            }
            if tokens.last().is_some_and(|s| s == "return")
                && source[start_comment..position].contains(['\n', '\r'])
            {
                return Err("return line breaks are unsupported".into());
            }
            position += 2;
            continue;
        }
        let start = position;
        if bytes[position..].starts_with(b"++")
            || bytes[position..].starts_with(b"--")
            || bytes[position..].starts_with(b"**")
        {
            return Err("increment/decrement/exponent operators are unsupported".into());
        }
        if b == b'\'' || b == b'"' {
            position += 1;
            while position < bytes.len() && bytes[position] != b {
                if bytes[position] == b'\\' || !bytes[position].is_ascii() {
                    return Err("escaped/non-ASCII strings are unsupported".into());
                }
                position += 1;
            }
            if position == bytes.len() {
                return Err("unterminated string".into());
            }
            position += 1;
        } else if b.is_ascii_alphabetic()
            || b == b'_'
            || (b == b'.' && bytes.get(position + 1).is_some_and(u8::is_ascii_alphabetic))
        {
            position += 1;
            while position < bytes.len()
                && (bytes[position].is_ascii_alphanumeric()
                    || matches!(bytes[position], b'_' | b'.'))
            {
                position += 1;
            }
        } else if b.is_ascii_digit() || b == b'.' {
            position += 1;
            while position < bytes.len()
                && (bytes[position].is_ascii_digit()
                    || bytes[position] == b'.'
                    || matches!(bytes[position], b'e' | b'E')
                    || (matches!(bytes[position], b'+' | b'-')
                        && matches!(bytes[position - 1], b'e' | b'E')))
            {
                position += 1;
            }
        } else if b"{}();,=+-*/%".contains(&b) {
            position += 1;
        } else {
            return Err(format!("unsupported syntax at byte {position}"));
        }
        tokens.push(source[start..position].to_owned());
        if tokens.len() > 512 {
            return Err("token budget exceeded".into());
        }
    }
    Ok(tokens)
}

struct Parser<'a> {
    tokens: Vec<String>,
    position: usize,
    locals: BTreeMap<String, usize>,
    immutable: BTreeSet<String>,
    assigned: BTreeSet<usize>,
    vector: bool,
    constants: &'a BTreeMap<String, f64>,
    depth: usize,
}

pub(crate) fn attach_scripts(
    graph: &mut crate::SceneGraph,
    scene_json: &str,
    properties: &BTreeMap<String, String>,
) -> Result<(), crate::SceneParseError> {
    let root: serde_json::Value = serde_json::from_str(scene_json)?;
    let mut bindings = Vec::new();
    fn walk(
        value: &serde_json::Value,
        path: &str,
        bindings: &mut Vec<(String, String)>,
        bytes: &mut usize,
    ) -> Result<(), String> {
        match value {
            serde_json::Value::Object(object) => {
                if let Some(script) = object.get("script") {
                    let source = script.as_str().ok_or("script source must be a string")?;
                    *bytes = bytes.saturating_add(source.len());
                    if bindings.len() >= MAX_BINDINGS || *bytes > MAX_SCENE_SOURCE_BYTES {
                        return Err(
                            "scene script count/source budget exceeded; all scripts disabled"
                                .into(),
                        );
                    }
                    bindings.push((path.into(), source.into()));
                }
                for (key, value) in object {
                    walk(value, &format!("{path}.{key}"), bindings, bytes)?;
                }
            }
            serde_json::Value::Array(array) => {
                for (i, value) in array.iter().enumerate() {
                    walk(value, &format!("{path}[{i}]"), bindings, bytes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    if let Err(reason) = walk(&root, "scene", &mut bindings, &mut 0) {
        graph.unsupported_features.push(crate::UnsupportedFeature {
            path: "scene.script".into(),
            feature: format!("SceneScript: {reason}"),
        });
        return Ok(());
    }
    for (path, source) in bindings {
        let mut accepted = None;
        for (index, node) in graph.nodes.iter().enumerate() {
            for field in ["alpha", "origin", "scale"] {
                if path == format!("scene.objects[{index}].{field}") {
                    let raw = &root["objects"][index];
                    let result = (|| {
                        if !matches!(
                            node.kind,
                            crate::SceneNodeKind::Image(_)
                                | crate::SceneNodeKind::Video(_)
                                | crate::SceneNodeKind::Text(_)
                                | crate::SceneNodeKind::Container
                        ) || node.audio_visualizer.is_some()
                            || node.backdrop_fade.is_some()
                            || node.dynamic_scale.is_some()
                            || (field == "alpha"
                                && matches!(node.kind, crate::SceneNodeKind::Container))
                        {
                            return Err("unsupported script consumer".into());
                        }
                        if raw[field].get("animation").is_some() {
                            return Err("combined script/timeline is unsupported".into());
                        }
                        let tokens = tokenize(&source)?;
                        let mut constants = BTreeMap::new();
                        for (key, value) in properties {
                            if let Ok(n) = value.parse::<f64>()
                                && n.is_finite()
                            {
                                constants.insert(format!("engine.userProperties.{key}"), n);
                            }
                        }
                        snapshot(&mut constants, "thisLayer", node, raw, Some(field));
                        for window in tokens.windows(3) {
                            if window[0] != "thisScene.getLayer" || window[1] != "(" {
                                continue;
                            }
                            let name = &window[2];
                            if name.len() < 2 || !name.starts_with(['\'', '"']) {
                                continue;
                            }
                            let name = &name[1..name.len() - 1];
                            let matches: Vec<_> = graph
                                .nodes
                                .iter()
                                .enumerate()
                                .filter(|(_, n)| n.name == name)
                                .collect();
                            if matches.len() == 1 {
                                let (i, n) = matches[0];
                                snapshot(
                                    &mut constants,
                                    &format!("layer:{name}"),
                                    n,
                                    &root["objects"][i],
                                    (i == index).then_some(field),
                                );
                            }
                        }
                        ScriptProgram::compile(&source, field != "alpha", &constants)
                    })();
                    accepted = Some((index, field, result));
                }
            }
        }
        match accepted {
            Some((index, field, Ok(program))) => {
                graph.nodes[index]
                    .transform
                    .scripts
                    .insert(field.into(), program);
            }
            Some((_, _, Err(reason))) => {
                graph.unsupported_features.push(crate::UnsupportedFeature {
                    path,
                    feature: format!("SceneScript: {reason}"),
                })
            }
            None => {
                // Preserve the pre-existing bounded audio-pattern recognition.
                let recognized = graph.nodes.iter().enumerate().any(|(i, n)| {
                    n.audio_visualizer.is_some() && path == format!("scene.objects[{i}].visible")
                });
                if !recognized {
                    graph.unsupported_features.push(crate::UnsupportedFeature {
                        path,
                        feature: "SceneScript: unsupported property/event consumer".into(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn snapshot(
    constants: &mut BTreeMap<String, f64>,
    prefix: &str,
    node: &crate::SceneNode,
    raw: &serde_json::Value,
    own_binding: Option<&str>,
) {
    fn dynamic(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(object) => {
                object.contains_key("script")
                    || object.contains_key("animation")
                    || object.values().any(dynamic)
            }
            serde_json::Value::Array(array) => array.iter().any(dynamic),
            _ => false,
        }
    }
    if node.audio_visualizer.is_some()
        || node.dynamic_scale.is_some()
        || node.backdrop_fade.is_some()
        || raw.as_object().is_some_and(|object| {
            object
                .iter()
                .any(|(key, value)| Some(key.as_str()) != own_binding && dynamic(value))
        })
    {
        return;
    }
    let static_field =
        |field: &str| raw[field].get("script").is_none() && raw[field].get("animation").is_none();
    for (field, vector) in [
        ("origin", node.transform.origin),
        ("scale", node.transform.scale),
    ] {
        if static_field(field) {
            let default = if field == "scale" { 1. } else { 0. };
            let vector = vector.unwrap_or(crate::Vec3 {
                x: default,
                y: default,
                z: default,
            });
            for (axis, n) in [("x", vector.x), ("y", vector.y), ("z", vector.z)] {
                constants.insert(format!("{prefix}.{field}.{axis}"), f64::from(n));
            }
        }
    }
    if static_field("alpha") {
        constants.insert(
            format!("{prefix}.alpha"),
            f64::from(node.transform.opacity.unwrap_or(1.)),
        );
    }
    if static_field("size")
        && let Some(size) = node.transform.size
    {
        constants.insert(format!("{prefix}.size.x"), f64::from(size.x));
        constants.insert(format!("{prefix}.size.y"), f64::from(size.y));
    }
}
impl Parser<'_> {
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.position).map(String::as_str)
    }
    fn next(&mut self) -> Result<&str, String> {
        let index = self.position;
        self.position += 1;
        self.tokens
            .get(index)
            .map(String::as_str)
            .ok_or_else(|| "unexpected end of script".into())
    }
    fn take(&mut self, expected: &str) -> Result<(), String> {
        if self.next()? == expected {
            Ok(())
        } else {
            Err(format!(
                "expected {expected}; unsupported SceneScript syntax/API"
            ))
        }
    }
    fn register(&self, name: &str) -> Option<usize> {
        match name {
            "value" if !self.vector => Some(0),
            "value.x" if self.vector => Some(0),
            "value.y" if self.vector => Some(1),
            "value.z" if self.vector => Some(2),
            _ => self.locals.get(name).copied(),
        }
    }
    fn expression(&mut self, minimum: u8) -> Result<Vec<Op>, String> {
        self.depth += 1;
        if self.depth > 32 {
            return Err("expression depth budget exceeded".into());
        }
        let token = self.next()?.to_owned();
        let mut code = match token.as_str() {
            "(" => {
                let code = self.expression(0)?;
                self.take(")")?;
                code
            }
            "-" | "+" => {
                let mut code = self.expression(3)?;
                if token == "-" {
                    code.push(Op::Negate);
                }
                code
            }
            "engine.runtime" => vec![Op::Time],
            "Math.PI" => vec![Op::Number(std::f64::consts::PI)],
            "Math.sin" | "Math.cos" | "Math.abs" | "Math.floor" | "Math.ceil" | "Math.round"
            | "Math.min" | "Math.max" => {
                self.take("(")?;
                let mut code = self.expression(0)?;
                if matches!(token.as_str(), "Math.min" | "Math.max") {
                    self.take(",")?;
                    code.extend(self.expression(0)?);
                }
                self.take(")")?;
                code.push(Op::Function(token));
                code
            }
            "thisScene.getLayer" => {
                self.take("(")?;
                let name = self.next()?.to_owned();
                self.take(")")?;
                let member = self.next()?.to_owned();
                if !name.starts_with(['\'', '"']) || !member.starts_with('.') {
                    return Err(
                        "getLayer requires a literal name and a static numeric member".into(),
                    );
                }
                let key = format!("layer:{}{}", &name[1..name.len() - 1], member);
                vec![Op::Number(*self.constants.get(&key).ok_or_else(|| {
                    format!("unavailable or dynamic scene object: {key}")
                })?)]
            }
            _ => {
                if let Ok(n) = token.parse::<f64>() {
                    if !n.is_finite() {
                        return Err("non-finite literal".into());
                    }
                    vec![Op::Number(n)]
                } else if let Some(index) = self.register(&token) {
                    if index < 3 && !self.assigned.contains(&index) {
                        return Err("stateful previous-value reads are unsupported; assign absolute values from scene time/properties".into());
                    }
                    vec![Op::Register(index)]
                } else if let Some(n) = self.constants.get(&token) {
                    vec![Op::Number(*n)]
                } else {
                    return Err(format!("unsupported capability or identifier: {token}"));
                }
            }
        };
        loop {
            let precedence = match self.peek() {
                Some("+" | "-") => 1,
                Some("*" | "/" | "%") => 2,
                _ => break,
            };
            if precedence < minimum {
                break;
            }
            let operator = self.next()?.chars().next().unwrap();
            code.extend(self.expression(precedence + 1)?);
            code.push(Op::Binary(operator));
            if code.len() > MAX_INSTRUCTIONS {
                return Err("instruction budget exceeded".into());
            }
        }
        self.depth -= 1;
        Ok(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar(body: &str) -> Result<ScriptProgram, String> {
        ScriptProgram::compile(
            &format!("export function update(value) {{ {body} }}"),
            false,
            &BTreeMap::new(),
        )
    }

    #[test]
    fn arithmetic_matches_javascript_precedence_and_math() {
        for (body, expected) in [
            ("return 1+2*3;", 7.),
            ("return (1+2)*3;", 9.),
            ("return 7-4-1;", 2.),
            ("return -7%3;", -1.),
            ("return Math.round(-1.5);", -1.),
            ("const a = 3; let b = a * 2; b = b+1; return b;", 7.),
            ("return Math.min(2,Math.max(1,Math.abs(-3)));", 2.),
            ("return Math.floor(1.9)+Math.ceil(1.1)+Math.cos(0);", 4.),
        ] {
            assert_eq!(
                scalar(body).unwrap().evaluate([0.; 3], 0.).unwrap(),
                [expected; 3],
                "{body}"
            );
        }
        let program = scalar("return Math.sin(engine.runtime); ").unwrap();
        assert!(
            (program
                .evaluate([0.; 3], std::f64::consts::FRAC_PI_2)
                .unwrap()[0]
                - 1.)
                .abs()
                < 1e-6
        );
    }

    #[test]
    fn rejects_unbounded_or_unavailable_language_and_host_capabilities() {
        for body in [
            "while(1) {} return 1;",
            "for(;;) {} return 1;",
            "return update(value);",
            "return fetch('file:///etc/passwd');",
            "return require('fs');",
            "return eval('1');",
            "return Function('return 1')();",
            "return engine.setTimeout(1);",
            "return engine.frametime;",
            "return Math.random();",
            "return value * 0.5;",
            "return 1++2;",
            "return 1--2;",
            "return --1;",
            "let a=1; return a++ + a;",
            "let arguments=1; return arguments;",
            "value = value + 1; return value;",
            "throw 1; return 1;",
            "return new Date();",
            "return shared.x;",
            "thisLayer.alpha = 0; return value;",
            "const a=1; a=2; return a;",
            "let return=1; return return;",
            "return\n1;",
            "return /*\n*/ 1;",
            "return 1; } export function destroy(value) { return 0;",
        ] {
            assert!(scalar(body).is_err(), "admitted {body}");
        }
        assert!(
            ScriptProgram::compile(&" ".repeat(MAX_SOURCE_BYTES + 1), false, &BTreeMap::new())
                .is_err()
        );
        assert!(scalar(&format!("return {}1{};", "(".repeat(33), ")".repeat(33))).is_err());
        assert!(scalar(&format!("{} return 1;", "value = 1;".repeat(33))).is_err());
        assert!(scalar(&format!("return {};", vec!["1"; 100].join("+"))).is_err());
    }

    #[test]
    fn arithmetic_faults_and_corrupt_serialized_programs_are_bounded() {
        for body in [
            "return 1/0;",
            "return 0/0;",
            "return 1000001;",
            "return 1e308*1e308;",
        ] {
            assert!(scalar(body).unwrap().evaluate([1.; 3], 0.).is_err());
        }
        let mut program = scalar("return value;").unwrap();
        assert!(program.evaluate([1.; 3], f64::NAN).is_err());
        program.result[0] = vec![Op::Register(REGISTERS)];
        assert!(program.evaluate([1.; 3], 0.).is_err());
        program.result[0] = vec![Op::Binary('+')];
        assert!(program.evaluate([1.; 3], 0.).is_err());
    }

    #[test]
    fn graph_properties_static_objects_and_opt_in_have_explicit_boundaries() {
        let scene = serde_json::json!({"objects":[
            {"id":1,"name":"Anchor","image":"a","origin":"12 34 0","size":"10 20"},
            {"id":2,"image":"a","origin":{"value":"1 2 0","script":"export function update(value) { value.x = thisScene.getLayer('Anchor').origin.x + engine.userProperties.offset; value.y = thisLayer.size.y; return value; }"},"size":"10 20"}
        ]}).to_string();
        let props = BTreeMap::from([("offset".into(), "5".into())]);
        let graph = crate::parse_scene_graph_with_scripts(&scene, &props, &props, true).unwrap();
        let program = &graph.nodes[1].transform.scripts["origin"];
        assert_eq!(program.evaluate([1., 2., 0.], 2.).unwrap(), [17., 20., 0.]);
        assert!(
            crate::parse_scene_graph_with_properties(&scene, &props)
                .unwrap()
                .nodes[1]
                .transform
                .scripts
                .is_empty()
        );
        let mut json: serde_json::Value = serde_json::from_str(&scene).unwrap();
        json["objects"][0]["origin"] = serde_json::json!({"value":"12 34 0","script":"export function update(value) { return value; }"});
        let graph =
            crate::parse_scene_graph_with_scripts(&json.to_string(), &props, &props, true).unwrap();
        assert!(graph.nodes[1].transform.scripts.is_empty());
        assert!(
            graph
                .unsupported_features
                .iter()
                .any(|f| f.feature.contains("dynamic scene object"))
        );
        json["objects"][0]["origin"] = "12 34 0".into();
        let duplicate = json["objects"][0].clone();
        json["objects"].as_array_mut().unwrap().push(duplicate);
        let graph =
            crate::parse_scene_graph_with_scripts(&json.to_string(), &props, &props, true).unwrap();
        assert!(graph.nodes[1].transform.scripts.is_empty());
    }

    #[test]
    fn scene_budget_disables_every_binding_without_partial_execution() {
        let node = serde_json::json!({"image":"a","alpha":{"value":1,"script":"export function update(value) { return 0; }"}});
        let scene = serde_json::json!({"objects":vec![node;MAX_BINDINGS+1]}).to_string();
        let graph =
            crate::parse_scene_graph_with_scripts(&scene, &BTreeMap::new(), &BTreeMap::new(), true)
                .unwrap();
        assert!(graph.nodes.iter().all(|n| n.transform.scripts.is_empty()));
        assert!(
            graph
                .unsupported_features
                .iter()
                .any(|f| f.feature.contains("budget exceeded"))
        );
    }

    #[test]
    fn mixed_scene_reports_capabilities_without_awarding_l4() {
        let scene = serde_json::json!({"objects":[
            {"image":"a","alpha":{"value":1,"script":"export function update(value) { return 0.5; }"}},
            {"image":"a","origin":{"value":"1 2 0","script":"export function update(value) { return engine.setTimeout(1); }"}}
        ]}).to_string();
        let meta = crate::analyse_scene(&scene).unwrap();
        assert_eq!(meta.bounded_script_count, 1);
        let report = crate::compute_compatibility(&meta);
        assert_ne!(report.level, crate::CompatibilityLevel::L4);
        assert!(
            report
                .supported_features
                .iter()
                .any(|f| f.contains("Restricted SceneScript"))
        );
        assert!(
            report
                .unsupported_features
                .iter()
                .any(|f| f.contains("engine.setTimeout"))
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|f| f.contains("not full SceneScript"))
        );
    }

    #[test]
    fn comments_are_not_executed_and_malformed_source_never_panics() {
        let program = ScriptProgram::compile("'use strict'; /* fetch('secret'); */ export function update(value) { // while(true){}\n return 0.5; }",false,&BTreeMap::new()).unwrap();
        assert_eq!(program.evaluate([1.; 3], 0.).unwrap(), [0.5; 3]);
        for source in [
            "",
            "/*",
            "'",
            "é",
            "export function update(value) {",
            "export function update(value) { return 1e+; }",
        ] {
            assert!(ScriptProgram::compile(source, false, &BTreeMap::new()).is_err());
        }
    }
}
