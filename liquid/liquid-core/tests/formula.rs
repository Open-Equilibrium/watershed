#![allow(clippy::unwrap_used)]

use std::collections::HashMap;

use liquid_core::{Failure, Formula, MAX_FORMULA, MAX_NESTING, Operand, Reference, Resolve, Value};

const AMOUNT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const CATEGORY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.cccccccccccccccccccccccccccccccc";
const LEFT: &str = "dddddddddddddddddddddddddddddddd";

struct Sheet(HashMap<String, Operand>);

impl Sheet {
    fn new() -> Self {
        let n = Value::Number;
        let t = |text: &str| Value::Text(text.into());
        Self(HashMap::from([
            (
                AMOUNT.into(),
                Operand::Range(vec![n(900.0), n(84.5), Value::Blank, n(300.0), t("n/a")]),
            ),
            (
                CATEGORY.into(),
                Operand::Range(vec![t("Rent"), t("Food"), t("Food"), t("rent"), t("Fun")]),
            ),
            (LEFT.into(), Operand::Value(n(715.5))),
        ]))
    }
}

impl Resolve for Sheet {
    fn resolve(&mut self, reference: &Reference) -> Operand {
        self.0
            .get(&reference.ids().join("."))
            .cloned()
            .unwrap_or(Operand::Value(Value::Error(Failure::Ref)))
    }
}

fn expand(source: &str) -> String {
    [("$A", AMOUNT), ("$C", CATEGORY), ("$L", LEFT)]
        .iter()
        .fold(source.to_string(), |s, (key, id)| {
            s.replace(key, &format!("{{{id}}}"))
        })
}

fn run(source: &str) -> Value {
    Formula::parse(&expand(source))
        .unwrap()
        .evaluate(&mut Sheet::new())
}

fn shown(source: &str) -> String {
    run(source).to_string()
}

#[test]
fn computes_as_excel_does() {
    for (source, expected) in [
        ("1 + 2 * 3", "7"),
        ("(1 + 2) * 3", "9"),
        ("2 ^ 3 ^ 2", "64"),
        ("-2 ^ 2", "4"),
        ("2 − 5", "-3"),
        ("50%", "0.5"),
        ("10 / 4", "2.5"),
        ("0.1 + 0.2", "0.3"),
        ("1 / 3", "0.333333333333333"),
        ("2 ^ 0.5 * 2 ^ 0.5", "2"),
        ("1e3 + 1", "1001"),
        ("\"Total: \" & 3 & \" €\"", "Total: 3 €"),
        ("\"Say \"\"hi\"\"\"", "Say \"hi\""),
        ("\"a\" = \"A\"", "TRUE"),
        ("2 <> 3", "TRUE"),
        ("\"10\" + 5", "15"),
        ("TRUE + 1", "2"),
        ("1 / 0", "#DIV/0!"),
        ("\"x\" * 2", "#VALUE!"),
        ("SQRT(-1)", "#NUM!"),
        ("NOPE(1)", "#NAME?"),
        ("{eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee} + 1", "#REF!"),
        ("1 + 1 / 0 + SQRT(-1)", "#DIV/0!"),
    ] {
        assert_eq!(shown(source), expected, "{source}");
    }
}

#[test]
fn excel_functions_read_references_and_ranges() {
    for (source, expected) in [
        ("2000 − SUM($A)", "715.5"),
        ("$L * 2", "1431"),
        ("AVERAGE($A)", "428.166666666667"),
        ("MIN($A, 50)", "50"),
        ("MAX($A)", "900"),
        ("COUNT($A)", "3"),
        ("COUNTA($A)", "4"),
        ("COUNTBLANK($A)", "1"),
        ("SUMIF($C, \"rent\", $A)", "1200"),
        ("SUMIF($A, \">100\")", "1200"),
        ("COUNTIF($C, \"<>Food\")", "3"),
        ("COUNTIF($C, \"F*\")", "3"),
        ("AVERAGEIF($C, \"Food\", $A)", "84.5"),
        ("AVERAGEIF($C, \"None\", $A)", "#DIV/0!"),
        ("SUM($A) + $A", "#VALUE!"),
        ("PRODUCT(2, 3, 4)", "24"),
        ("MEDIAN(5, 1, 3, 2)", "2.5"),
        ("IF($L > 0, \"left\", 1 / 0)", "left"),
        ("IF(FALSE, 1)", "FALSE"),
        ("IFERROR(1 / 0, \"none\")", "none"),
        ("AND(TRUE, 1, 2 > 1)", "TRUE"),
        ("OR(FALSE, 0)", "FALSE"),
        ("NOT(0)", "TRUE"),
        (
            "ROUND(2.5, 0) & ROUND(-2.5, 0) & ROUND(1.005, 2)",
            "3-31.01",
        ),
        ("ROUNDUP(1.21, 1) & \"|\" & ROUNDDOWN(-1.29, 1)", "1.3|-1.2"),
        ("ABS(-3) + INT(-2.5) + MOD(-7, 3) + POWER(2, 10)", "1026"),
        ("LEN(\"Liquid\") & UPPER(\"a\") & LOWER(\"B\")", "6Ab"),
        (
            "TRIM(\"  a   b \") & LEFT(\"Rent\", 2) & RIGHT(\"Rent\") & MID(\"Liquid\", 2, 3)",
            "a bRetiqu",
        ),
        ("CONCAT(\"a\", 1, TRUE)", "a1TRUE"),
        (
            "ISBLANK(INDEX($A, 3)) & ISNUMBER(1) & ISTEXT(1) & ISERROR(1 / 0)",
            "TRUETRUEFALSETRUE",
        ),
        ("ROUND(1, 0, 0)", "#VALUE!"),
    ] {
        assert_eq!(shown(source), expected, "{source}");
    }
}

#[test]
fn lists_its_references_and_rejects_what_it_cannot_read() {
    let formula = Formula::parse(&expand("SUM($A) - $L")).unwrap();
    let ids: Vec<_> = formula.references().map(|r| r.ids().join(".")).collect();
    assert_eq!(ids, [AMOUNT, LEFT]);

    let deep = format!(
        "{}1{}",
        "(".repeat(MAX_NESTING + 1),
        ")".repeat(MAX_NESTING + 1)
    );
    for source in [
        "",
        "1 +",
        "(1",
        "SUM(1,",
        "\"open",
        "{abc}",
        "1 2",
        &deep,
        &"1".repeat(MAX_FORMULA + 1),
    ] {
        assert!(Formula::parse(source).is_err(), "{source:?}");
    }
    let nested = format!("{}1{}", "(".repeat(MAX_NESTING), ")".repeat(MAX_NESTING));
    assert_eq!(shown(&nested), "1");
}
