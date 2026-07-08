//! Recursive-descent parser for the `join` expression mini-language used in
//! `relationships[*].join`.
//!
//! Grammar:
//!
//! ```text
//! join     := conjunct ("AND" conjunct)*
//! conjunct := qcol op qcol
//! qcol     := IDENT "." IDENT
//! op       := "=" | "==" | ">=" | "<=" | ">" | "<"
//! IDENT    := [A-Za-z_][A-Za-z0-9_]*
//! ```
//!
//! `AND` is matched case-insensitively. Whitespace is permitted between
//! tokens. The parser tracks byte offsets within the input string so we can
//! point diagnostics at the failing token.

#[derive(Debug, Clone)]
pub struct JoinExpr {
    pub conjuncts: Vec<JoinConjunct>,
}

#[derive(Debug, Clone)]
pub struct JoinConjunct {
    pub lhs: QCol,
    pub op: JoinOp,
    pub rhs: QCol,
}

#[derive(Debug, Clone)]
pub struct QCol {
    pub table: String,
    pub column: String,
    /// Byte offset of the qualified column within the join string.
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinOp {
    Eq,
    Ge,
    Le,
    Gt,
    Lt,
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    /// Byte offset of the failing token (or end-of-string) within the join
    /// expression.
    pub at: usize,
}

impl JoinExpr {
    pub fn parse(input: &str) -> Result<JoinExpr, ParseError> {
        let mut p = Parser::new(input);
        let mut conjuncts = vec![p.parse_conjunct()?];
        loop {
            p.skip_ws();
            if p.is_eof() {
                break;
            }
            p.expect_keyword("and")?;
            conjuncts.push(p.parse_conjunct()?);
        }
        Ok(JoinExpr { conjuncts })
    }

    /// Distinct table names referenced by any `qcol` in the expression.
    /// Order matches first-appearance order in the source so diagnostics are
    /// stable.
    pub fn tables(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for c in &self.conjuncts {
            for q in [&c.lhs, &c.rhs] {
                if !out.iter().any(|t| *t == q.table) {
                    out.push(&q.table);
                }
            }
        }
        out
    }

    /// All qualified column references, in source order.
    pub fn qcols(&self) -> impl Iterator<Item = &QCol> {
        self.conjuncts
            .iter()
            .flat_map(|c| [&c.lhs, &c.rhs].into_iter())
    }

    /// The join's two positional sides as `(probe_table, other_table)`.
    /// The FIRST conjunct defines which table is "left"; `probe_left`
    /// picks which side is being probed (the "many" side in a D05 check).
    /// Positional, so a self-join still has two distinguishable sides.
    pub fn sides(&self, probe_left: bool) -> (&str, &str) {
        // parse() never produces an empty conjunct list
        let first = &self.conjuncts[0];
        if probe_left {
            (&first.lhs.table, &first.rhs.table)
        } else {
            (&first.rhs.table, &first.lhs.table)
        }
    }

    /// Every conjunct re-read from the probed side: `probe OP other`.
    ///
    /// This is the one place that owns D05 orientation semantics — the
    /// data validator (rich.rs) and the dummy-data planner both consume
    /// it, so they cannot drift apart:
    /// * conjuncts written the other way round (`b.lo <= a.ts` for
    ///   `a.ts >= b.lo`) are canonicalized against the first conjunct's
    ///   left table before orienting — same predicate, operator mirrored
    /// * table names match ASCII-case-insensitively, because duckdb
    ///   identifiers fold case (a self-join is already canonical: both
    ///   names are the same table, so orientation stays positional)
    /// * probing the right side mirrors each operator again so the result
    ///   always reads probe-side first
    pub fn oriented(&self, probe_left: bool) -> Vec<OrientedConjunct<'_>> {
        let left_table = &self.conjuncts[0].lhs.table;
        let mut out = Vec::new();
        for conj in &self.conjuncts {
            // canonicalize: lhs on the join's left table
            let (lhs, op, rhs) = if conj.lhs.table.eq_ignore_ascii_case(left_table) {
                (&conj.lhs, conj.op, &conj.rhs)
            } else {
                (&conj.rhs, flip_op(conj.op), &conj.lhs)
            };
            // orient for the probe
            let (probe, op, other) = if probe_left {
                (lhs, op, rhs)
            } else {
                (rhs, flip_op(op), lhs)
            };
            out.push(OrientedConjunct { probe, op, other });
        }
        out
    }
}

/// One join conjunct read from the probed side: `probe OP other`.
/// Produced by [`JoinExpr::oriented`].
#[derive(Debug, Clone, Copy)]
pub struct OrientedConjunct<'a> {
    pub probe: &'a QCol,
    pub op: JoinOp,
    pub other: &'a QCol,
}

/// Mirror a comparison so its operands can swap sides: `a >= b` and
/// `b <= a` are the same predicate. Equality is its own mirror.
pub fn flip_op(op: JoinOp) -> JoinOp {
    match op {
        JoinOp::Eq => JoinOp::Eq,
        JoinOp::Ge => JoinOp::Le,
        JoinOp::Le => JoinOp::Ge,
        JoinOp::Gt => JoinOp::Lt,
        JoinOp::Lt => JoinOp::Gt,
    }
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            src: s.as_bytes(),
            pos: 0,
        }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn err(&self, msg: impl Into<String>) -> ParseError {
        ParseError {
            message: msg.into(),
            at: self.pos,
        }
    }

    fn parse_conjunct(&mut self) -> Result<JoinConjunct, ParseError> {
        let lhs = self.parse_qcol()?;
        self.skip_ws();
        let op = self.parse_op()?;
        self.skip_ws();
        let rhs = self.parse_qcol()?;
        Ok(JoinConjunct { lhs, op, rhs })
    }

    fn parse_qcol(&mut self) -> Result<QCol, ParseError> {
        self.skip_ws();
        let start = self.pos;
        let table = self.parse_ident()?;
        if self.peek() != Some(b'.') {
            return Err(self.err("expected `.` after table name"));
        }
        self.pos += 1;
        let column = self.parse_ident()?;
        Ok(QCol {
            table,
            column,
            start,
            end: self.pos,
        })
    }

    fn parse_ident(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        match self.peek() {
            Some(b) if b.is_ascii_alphabetic() || b == b'_' => self.pos += 1,
            _ => return Err(self.err("expected identifier")),
        }
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || b == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        Ok(std::str::from_utf8(&self.src[start..self.pos])
            .expect("identifier bytes are ASCII")
            .to_string())
    }

    fn parse_op(&mut self) -> Result<JoinOp, ParseError> {
        // Order matters: longer operators first.
        for (lit, op) in [
            (">=", JoinOp::Ge),
            ("<=", JoinOp::Le),
            ("==", JoinOp::Eq),
            ("=", JoinOp::Eq),
            (">", JoinOp::Gt),
            ("<", JoinOp::Lt),
        ] {
            if self.src[self.pos..].starts_with(lit.as_bytes()) {
                self.pos += lit.len();
                return Ok(op);
            }
        }
        Err(self.err("expected one of `=`, `>=`, `<=`, `>`, `<`"))
    }

    fn expect_keyword(&mut self, kw: &str) -> Result<(), ParseError> {
        let end = self.pos + kw.len();
        if end > self.src.len() {
            return Err(self.err(format!("expected `{}`", kw.to_uppercase())));
        }
        let slice = &self.src[self.pos..end];
        if !slice.eq_ignore_ascii_case(kw.as_bytes()) {
            return Err(self.err(format!("expected `{}`", kw.to_uppercase())));
        }
        // Keyword must be followed by a non-identifier character (so we don't
        // match `andante` as `AND` + `ante`).
        if let Some(&b) = self.src.get(end)
            && (b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(self.err(format!("expected `{}`", kw.to_uppercase())));
        }
        self.pos = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> JoinExpr {
        JoinExpr::parse(s)
            .unwrap_or_else(|e| panic!("parse({s:?}) failed: {} at {}", e.message, e.at))
    }

    #[test]
    fn simple_equality() {
        let j = parse("food.fdc_id = food_nutrient.fdc_id");
        assert_eq!(j.conjuncts.len(), 1);
        assert_eq!(j.conjuncts[0].lhs.table, "food");
        assert_eq!(j.conjuncts[0].lhs.column, "fdc_id");
        assert_eq!(j.conjuncts[0].op, JoinOp::Eq);
        assert_eq!(j.conjuncts[0].rhs.table, "food_nutrient");
        assert_eq!(j.tables(), vec!["food", "food_nutrient"]);
    }

    #[test]
    fn self_join() {
        let j = parse("otters.pup_number = otters.otter_no");
        assert_eq!(j.tables(), vec!["otters"]);
    }

    #[test]
    fn multi_conjunct_with_and() {
        let j = parse("t1.date >= t2.start AND t1.date <= t2.end");
        assert_eq!(j.conjuncts.len(), 2);
        assert_eq!(j.conjuncts[0].op, JoinOp::Ge);
        assert_eq!(j.conjuncts[1].op, JoinOp::Le);
        assert_eq!(j.tables(), vec!["t1", "t2"]);
    }

    #[test]
    fn and_is_case_insensitive() {
        let j = parse("a.x = b.y and a.z = b.w");
        assert_eq!(j.conjuncts.len(), 2);
    }

    #[test]
    fn leading_and_trailing_whitespace() {
        let j = parse("  a.x = b.y  ");
        assert_eq!(j.conjuncts.len(), 1);
    }

    #[test]
    fn accepts_double_equals() {
        // `==` is an alternate spelling of `=`. Useful because most
        // programming languages spell equality that way and the validator
        // shouldn't punish that habit.
        let j = parse("food.fdc_id == food_nutrient.fdc_id");
        assert_eq!(j.conjuncts.len(), 1);
        assert_eq!(j.conjuncts[0].op, JoinOp::Eq);
    }

    #[test]
    fn rejects_missing_dot() {
        let err = JoinExpr::parse("food = food_nutrient.fdc_id").unwrap_err();
        assert!(err.message.contains('.'));
    }

    #[test]
    fn rejects_unknown_operator() {
        let err = JoinExpr::parse("a.x ~ b.y").unwrap_err();
        assert!(err.message.contains('='));
    }

    #[test]
    fn rejects_trailing_and() {
        let err = JoinExpr::parse("a.x = b.y AND").unwrap_err();
        // After consuming `AND`, the parser tries to read a qcol.
        assert!(err.message.contains("identifier"));
    }

    #[test]
    fn rejects_andante_as_and() {
        // The bare keyword check must not greedily match identifier prefixes.
        let err = JoinExpr::parse("a.x = b.y andante c.z = d.w").unwrap_err();
        assert!(err.message.contains("AND"));
    }

    #[test]
    fn qcol_byte_spans_are_correct() {
        let s = "food.fdc_id = food_nutrient.fdc_id";
        let j = JoinExpr::parse(s).unwrap();
        let lhs = &j.conjuncts[0].lhs;
        assert_eq!(&s[lhs.start..lhs.end], "food.fdc_id");
        let rhs = &j.conjuncts[0].rhs;
        assert_eq!(&s[rhs.start..rhs.end], "food_nutrient.fdc_id");
    }

    #[test]
    fn all_operators_parse() {
        let cases = [
            ("a.x = b.y", JoinOp::Eq),
            ("a.x == b.y", JoinOp::Eq),
            ("a.x >= b.y", JoinOp::Ge),
            ("a.x <= b.y", JoinOp::Le),
            ("a.x > b.y", JoinOp::Gt),
            ("a.x < b.y", JoinOp::Lt),
        ];
        for (s, op) in cases {
            let j = parse(s);
            assert_eq!(j.conjuncts[0].op, op, "operator mismatch for {s:?}");
        }
    }

    #[test]
    fn two_char_operators_are_not_split() {
        // `>=` must win over `>`, otherwise the trailing `=` would be left for
        // the rhs qcol parse and the operator would be wrong.
        assert_eq!(parse("a.x >= b.y").conjuncts[0].op, JoinOp::Ge);
        assert_eq!(parse("a.x <= b.y").conjuncts[0].op, JoinOp::Le);
    }

    #[test]
    fn whitespace_around_operator_is_optional() {
        let j = parse("a.x=b.y");
        assert_eq!(j.conjuncts.len(), 1);
        assert_eq!(j.conjuncts[0].lhs.column, "x");
        assert_eq!(j.conjuncts[0].op, JoinOp::Eq);
        assert_eq!(j.conjuncts[0].rhs.column, "y");
    }

    #[test]
    fn three_conjuncts() {
        let j = parse("a.x = b.y AND c.z = d.w AND e.p = f.q");
        assert_eq!(j.conjuncts.len(), 3);
        assert_eq!(j.tables(), vec!["a", "b", "c", "d", "e", "f"]);
    }

    #[test]
    fn tables_dedup_in_first_appearance_order() {
        // `b` and `a` reappear in the second conjunct but must not be repeated.
        let j = parse("a.x = b.y AND b.z = a.w");
        assert_eq!(j.tables(), vec!["a", "b"]);
    }

    #[test]
    fn qcols_yields_all_refs_in_source_order() {
        let j = parse("a.x = b.y AND c.z = d.w");
        let cols: Vec<(&str, &str)> = j
            .qcols()
            .map(|q| (q.table.as_str(), q.column.as_str()))
            .collect();
        assert_eq!(cols, vec![("a", "x"), ("b", "y"), ("c", "z"), ("d", "w")]);
    }

    #[test]
    fn identifiers_may_contain_digits_underscores_and_leading_underscore() {
        let j = parse("_t1.col_2 = T_3.x9");
        assert_eq!(j.conjuncts[0].lhs.table, "_t1");
        assert_eq!(j.conjuncts[0].lhs.column, "col_2");
        assert_eq!(j.conjuncts[0].rhs.table, "T_3");
        assert_eq!(j.conjuncts[0].rhs.column, "x9");
    }

    #[test]
    fn rejects_empty_input() {
        let err = JoinExpr::parse("").unwrap_err();
        assert!(err.message.contains("identifier"));
        assert_eq!(err.at, 0);
    }

    #[test]
    fn rejects_whitespace_only_input() {
        let err = JoinExpr::parse("   ").unwrap_err();
        assert!(err.message.contains("identifier"));
    }

    #[test]
    fn rejects_identifier_starting_with_digit() {
        let err = JoinExpr::parse("1a.x = b.y").unwrap_err();
        assert!(err.message.contains("identifier"));
    }

    #[test]
    fn rejects_missing_column_after_dot() {
        let err = JoinExpr::parse("a. = b.y").unwrap_err();
        assert!(err.message.contains("identifier"));
    }

    #[test]
    fn rejects_missing_operator() {
        // After the lhs qcol the parser expects an operator, not another qcol.
        let err = JoinExpr::parse("a.x b.y").unwrap_err();
        assert!(err.message.contains('='));
    }

    #[test]
    fn rejects_missing_rhs() {
        let err = JoinExpr::parse("a.x =").unwrap_err();
        assert!(err.message.contains("identifier"));
    }

    #[test]
    fn rejects_two_conjuncts_without_and() {
        // A second conjunct must be separated by `AND`; bare juxtaposition is
        // an error pointing at the missing keyword.
        let err = JoinExpr::parse("a.x = b.y c.z = d.w").unwrap_err();
        assert!(err.message.contains("AND"));
    }

    #[test]
    fn error_offset_points_at_failing_token() {
        // The unknown operator sits at byte 4.
        let err = JoinExpr::parse("a.x ~ b.y").unwrap_err();
        assert_eq!(err.at, 4);
    }

    // --- orientation (shared by the D05 validator and the dummy-data planner) ---

    #[test]
    fn sides_follow_the_first_conjunct() {
        let j = parse("events.ts >= periods.lo AND events.ts <= periods.hi");
        assert_eq!(j.sides(true), ("events", "periods"));
        assert_eq!(j.sides(false), ("periods", "events"));
    }

    #[test]
    fn oriented_probe_left_keeps_canonical_conjuncts_as_written() {
        let j = parse("a.x >= b.y");
        let oc = j.oriented(true);
        assert_eq!(oc.len(), 1);
        assert_eq!(oc[0].probe.column, "x");
        assert_eq!(oc[0].op, JoinOp::Ge);
        assert_eq!(oc[0].other.column, "y");
    }

    #[test]
    fn oriented_probe_right_mirrors_every_operator() {
        // probing b reads the same predicate from b's side: `a.x >= b.y`
        // becomes `b.y <= a.x`
        let j = parse("a.x >= b.y AND a.z < b.w");
        let oc = j.oriented(false);
        assert_eq!(oc[0].probe.column, "y");
        assert_eq!(oc[0].op, JoinOp::Le);
        assert_eq!(oc[0].other.column, "x");
        assert_eq!(oc[1].probe.column, "w");
        assert_eq!(oc[1].op, JoinOp::Gt);
        assert_eq!(oc[1].other.column, "z");
    }

    #[test]
    fn oriented_canonicalizes_conjuncts_written_the_other_way_round() {
        // the second conjunct is written right-to-left: `b.lo <= a.ts` is
        // the same predicate as `a.ts >= b.lo`, and must orient like it
        let j = parse("a.ts <= b.hi AND b.lo <= a.ts");
        let oc = j.oriented(true);
        assert_eq!(oc[1].probe.column, "ts");
        assert_eq!(oc[1].op, JoinOp::Ge);
        assert_eq!(oc[1].other.column, "lo");
    }

    #[test]
    fn oriented_matches_tables_case_insensitively() {
        // duckdb identifiers fold case, so `A.p = a.q`'s second conjunct
        // spelled `A`/`a` differently still canonicalizes by table match —
        // the validator compares this way (names_eq) and the planner must
        // agree with it
        let j = parse("a.x = b.y AND B.q = A.p");
        let oc = j.oriented(true);
        assert_eq!(oc[1].probe.column, "p");
        assert_eq!(oc[1].other.column, "q");
    }

    #[test]
    fn oriented_self_join_is_positional() {
        // both sides name the same table: conjuncts are already canonical
        // (lhs is "the left side") and probing flips positionally
        let j = parse("otters.pup_number = otters.otter_no");
        let left = j.oriented(true);
        assert_eq!(left[0].probe.column, "pup_number");
        let right = j.oriented(false);
        assert_eq!(right[0].probe.column, "otter_no");
    }

    #[test]
    fn flip_op_mirrors_comparisons() {
        assert_eq!(flip_op(JoinOp::Eq), JoinOp::Eq);
        assert_eq!(flip_op(JoinOp::Ge), JoinOp::Le);
        assert_eq!(flip_op(JoinOp::Le), JoinOp::Ge);
        assert_eq!(flip_op(JoinOp::Gt), JoinOp::Lt);
        assert_eq!(flip_op(JoinOp::Lt), JoinOp::Gt);
    }
}
