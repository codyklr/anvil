//! Optional, in-process GLiNER filter interpreter. No network calls during inference.
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use candle_core::{DType, Device};
use gliner_rs::{ClassificationSpec, ExtractOptions, GLiNER2, Schema};
use serde_json::Value;

use crate::{ColumnType, Filter, FilterExpr, FilterOp, NlSchema};

pub const MODEL_REPO: &str = "fastino/gliner2.5-small-v1";
pub const MODEL_REVISION: &str = "7132dc4561c3f94563c6147e75ffa8ef34c4964a";
const FILES: &[&str] = &[
    "config.json",
    "encoder_config/config.json",
    "tokenizer.json",
    "model.safetensors",
];
const MIN_CONFIDENCE: f64 = 0.65;
/// Row scope classification labels. `one column` is the default when the
/// model does not emit a scope, so single-column behavior is unchanged.
const SCOPES: &[&str] = &["one column", "any column", "all columns"];
const OPERATORS: &[(&str, FilterOp)] = &[
    ("equals", FilterOp::Eq),
    ("not equal", FilterOp::Ne),
    ("contains", FilterOp::Contains),
    ("does not contain", FilterOp::NotContains),
    ("starts with", FilterOp::StartsWith),
    ("ends with", FilterOp::EndsWith),
    ("is empty", FilterOp::Empty),
    ("is not empty", FilterOp::NonEmpty),
    ("greater than", FilterOp::Gt),
    ("at least", FilterOp::Ge),
    ("less than", FilterOp::Lt),
    ("at most", FilterOp::Le),
];

struct Engine {
    model: GLiNER2,
    processor: gliner_rs::processor::Processor,
}

/// One loaded model per interpreter, with no queue of concurrent inference jobs.
#[derive(Default)]
pub struct Interpreter {
    engine: Mutex<Option<Engine>>,
}

impl Interpreter {
    pub fn interpret(
        &self,
        phrase: &str,
        schema: &NlSchema,
        values: &dyn Fn(usize) -> Vec<String>,
    ) -> Result<FilterExpr, String> {
        let mut extraction = extraction_schema(phrase, schema, values)?;
        let mut slot = self
            .engine
            .try_lock()
            .map_err(|_| "The local model is busy; retry shortly.".to_string())?;
        if slot.is_none() {
            let dir = model_dir()?;
            for file in FILES {
                if !dir.join(file).is_file() {
                    return Err("Ask model is not installed. Run anvil --setup-ask-model, or set ANVIL_GLINER_MODEL_DIR to a checkpoint directory.".into());
                }
            }
            let tokenizer = gliner_tokenizers::Tokenizer::from_file(dir.join("tokenizer.json"))
                .map_err(|e| format!("Cannot load Ask tokenizer: {e}"))?;
            let model = GLiNER2::load(&dir, &Device::Cpu, DType::F32)
                .map_err(|e| format!("Cannot load Ask model from {}: {e}", dir.display()))?;
            *slot = Some(Engine {
                model,
                processor: gliner_rs::processor::Processor::new(tokenizer),
            });
        }
        let engine = slot.as_ref().unwrap();
        // Count the actual schema + question tokens before inference. Never truncate
        // a question or silently omit columns to fit the encoder context.
        let mut input = engine
            .processor
            .prepare(phrase, &extraction, None)
            .map_err(|e| e.to_string())?;
        if input.input_ids.len() > 512 {
            // Enriched column descriptions can push a wide schema over the encoder
            // budget. Degrade to the raw-name schema rather than dropping columns or
            // truncating the question; deterministic value binding still resolves
            // implicit columns.
            extraction = raw_extraction_schema(schema, values);
            input = engine
                .processor
                .prepare(phrase, &extraction, None)
                .map_err(|e| e.to_string())?;
            if input.input_ids.len() > 512 {
                return Err("Question and column names exceed the model's 512-token limit. Use a shorter question or the filter bar.".into());
            }
        }
        let output = engine
            .model
            .extract(
                phrase,
                &extraction,
                &ExtractOptions {
                    include_confidence: true,
                    include_spans: true,
                    ..Default::default()
                },
            )
            .map_err(|e| format!("Ask inference failed: {e}"))?;
        #[cfg(test)]
        eprintln!("GLiNER output: {output}");
        decode(phrase, schema, &output, values)
    }
}

fn validate_bounds(phrase: &str, schema: &NlSchema) -> Result<(), String> {
    if phrase.trim().is_empty()
        || phrase.len() > 512
        || schema.columns.is_empty()
        || schema.columns.len() > 32
        || schema.columns.iter().map(|c| c.name.len()).sum::<usize>() > 1024
    {
        return Err("Ask supports a nonempty question up to 512 bytes and up to 32 columns (1024 bytes of column names). Use the filter bar for larger requests.".into());
    }
    // Conservative scope guard for this first single-condition backend. The
    // model cannot establish complete coverage of an arbitrary English query.
    let words: Vec<_> = phrase
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.iter().any(|w| {
        matches!(
            w.as_str(),
            "and"
                | "or"
                | "between"
                | "except"
                | "unless"
                | "without"
                | "but"
                | "also"
                | "then"
                | "count"
                | "sum"
                | "average"
                | "total"
                | "sort"
                | "top"
                | "bottom"
                | "latest"
                | "earliest"
        )
    }) || words
        .windows(2)
        .any(|w| w == ["how", "many"] || w == ["last", "month"] || w == ["this", "year"])
        || phrase.contains([',', ';', '\n', '&', '|'])
    {
        return Err("This model preview supports one filter condition. Use the filter bar for compound filters, dates, sorting, or totals.".into());
    }
    for (i, column) in schema.columns.iter().enumerate() {
        if column.name.trim().is_empty()
            || schema.columns[..i]
                .iter()
                .any(|c| c.name.eq_ignore_ascii_case(&column.name))
        {
            return Err(
                "Ask needs unique, nonempty column names. Use the filter bar for this file.".into(),
            );
        }
    }
    Ok(())
}

fn extraction_schema(
    phrase: &str,
    schema: &NlSchema,
    values: &dyn Fn(usize) -> Vec<String>,
) -> Result<Schema, String> {
    validate_bounds(phrase, schema)?;
    Ok(build_schema(schema, values, true))
}

/// The raw-name fallback used when enriched column descriptions would exceed
/// the encoder token budget. It keeps the deterministic value binding intact.
fn raw_extraction_schema(schema: &NlSchema, values: &dyn Fn(usize) -> Vec<String>) -> Schema {
    build_schema(schema, values, false)
}

fn build_schema(schema: &NlSchema, values: &dyn Fn(usize) -> Vec<String>, enrich: bool) -> Schema {
    let mut columns = ClassificationSpec::new(
        "column to filter",
        schema.columns.iter().map(|c| c.name.clone()),
    );
    if enrich {
        columns.prompt = Some(
            "Match the column whose data the question refers to, even when the column name is not mentioned."
                .into(),
        );
        for (index, column) in schema.columns.iter().enumerate() {
            let aliases: Vec<String> = schema
                .aliases
                .iter()
                .filter(|(_, target)| *target == index)
                .map(|(alias, _)| alias.clone())
                .collect();
            let samples = values(index);
            let description =
                column_description(&column.name, column.column_type, &aliases, &samples);
            columns
                .label_descriptions
                .push((column.name.clone(), description));
        }
    }
    let operators =
        ClassificationSpec::new("filter comparison", OPERATORS.iter().map(|(name, _)| *name));
    // Appended after the column and comparison specs so index-based callers
    // and tests keep seeing the column spec first. The descriptions steer the
    // model away from picking a specific column when the question is about
    // rows with an empty/filled column.
    let mut scope = ClassificationSpec::new("filter scope", SCOPES.iter().copied())
        .with_label_description(
            "one column",
            "The question filters one specific column, named or implied.",
        )
        .with_label_description(
            "any column",
            "The question asks for rows where at least one column, field, or cell matches, without naming a specific column.",
        )
        .with_label_description(
            "all columns",
            "The question requires every column to match, such as all columns being empty.",
        );
    scope.prompt = Some(
        "Decide whether the question is about one named column, any column, or all columns.".into(),
    );
    Schema::new()
        .classification(columns)
        .classification(operators)
        .classification(scope)
        .entity_with_description("filter value", "The literal text or number to match in the requested filter, excluding column names and comparison words")
}

/// A model-facing description for one column label: name, type, known synonym
/// phrasings, and up to five sampled values. Advisory only: it primes the
/// zero-shot classifier for indirect references such as "from Canada" but
/// never overrides deterministic value binding.
fn column_description(
    name: &str,
    kind: ColumnType,
    aliases: &[String],
    samples: &[String],
) -> String {
    let kind = match kind {
        ColumnType::Integer | ColumnType::Float => "number",
        ColumnType::Boolean => "boolean",
        ColumnType::Text => "text",
        ColumnType::Empty => "empty",
    };
    let mut description = format!("Column '{}' ({}).", sanitize(name), kind);
    if !aliases.is_empty() {
        let listed: Vec<String> = aliases.iter().take(5).map(|a| sanitize(a)).collect();
        description.push_str(&format!(" Synonyms may include: {}.", listed.join(", ")));
    }
    if !samples.is_empty() {
        let listed: Vec<String> = samples.iter().take(5).map(|s| sanitize(s)).collect();
        description.push_str(&format!(" Example values: {}.", listed.join(", ")));
    }
    cap_description(description, 160)
}

fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(48)
        .collect::<String>()
        .trim()
        .to_string()
}

fn cap_description(text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }
    let kept: String = text.chars().take(max_chars).collect();
    format!("{kept}…")
}

fn classified<'a>(output: &'a Value, key: &str) -> Result<&'a str, String> {
    let value = &output[key];
    confidence(value)?;
    value["label"]
        .as_str()
        .ok_or_else(|| format!("The model did not identify {key}. Try a more explicit question."))
}

fn confidence(value: &Value) -> Result<(), String> {
    if value["confidence"]
        .as_f64()
        .is_some_and(|p| p.is_finite() && (MIN_CONFIDENCE..=1.0).contains(&p))
    {
        Ok(())
    } else {
        Err("The model is uncertain. Name the column, comparison, and value explicitly.".into())
    }
}

fn decode(
    phrase: &str,
    schema: &NlSchema,
    output: &Value,
    values: &dyn Fn(usize) -> Vec<String>,
) -> Result<FilterExpr, String> {
    let label = classified(output, "filter comparison")?;
    let op = OPERATORS
        .iter()
        .find(|(name, _)| *name == label)
        .map(|(_, op)| *op)
        .ok_or("The model returned an unknown comparison.")?;
    let words: Vec<_> = phrase
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .collect();
    let negated = words
        .iter()
        .any(|w| matches!(w.as_str(), "not" | "never" | "isn" | "doesn" | "don"));
    if negated
        && !matches!(
            op,
            FilterOp::Ne | FilterOp::NotContains | FilterOp::NonEmpty
        )
    {
        return Err(
            "The prediction does not preserve the question's negation. Use an explicit filter."
                .into(),
        );
    }
    if negated
        && words.iter().any(|w| {
            matches!(
                w.as_str(),
                "start"
                    | "starts"
                    | "starting"
                    | "begin"
                    | "begins"
                    | "beginning"
                    | "end"
                    | "ends"
                    | "ending"
            )
        })
    {
        return Err("Negated prefix and suffix filters are not supported by this backend.".into());
    }
    let spans = output["entities"]["filter value"]
        .as_array()
        .ok_or("The model did not return filter values.")?;
    let value = if op.needs_value() {
        if spans.len() != 1 {
            return Err(
                "The model must identify exactly one filter value. Please rephrase.".into(),
            );
        }
        let extracted = &spans[0];
        confidence(extracted)?;
        let text = extracted["text"].as_str().ok_or("Missing filter value.")?;
        let start = extracted["start"].as_u64().ok_or("Missing value span.")? as usize;
        let end = extracted["end"].as_u64().ok_or("Missing value span.")? as usize;
        if start >= end
            || end > phrase.chars().count()
            || phrase
                .chars()
                .skip(start)
                .take(end - start)
                .collect::<String>()
                != text
        {
            return Err("The model value does not match the question.".into());
        }
        text.to_owned()
    } else {
        if !spans.is_empty() {
            return Err(
                "The model found a value for an empty-value comparison. Please rephrase.".into(),
            );
        }
        String::new()
    };
    // Scope is read leniently: a missing or unknown label means a normal
    // single-column predicate, so existing behavior is unchanged.
    let row_scope = match output["filter scope"]["label"].as_str() {
        Some("any column") => Some(true),
        Some("all columns") => Some(false),
        _ => None,
    };
    let column = if row_scope.is_some() {
        // Row scope does not reference one column; `column` is an unused
        // dummy kept at 0 so the node round-trips through describe/parse.
        0
    } else {
        resolve_column(schema, output, op, &value, values)?
    };
    if matches!(
        op,
        FilterOp::Gt | FilterOp::Ge | FilterOp::Lt | FilterOp::Le
    ) {
        let numeric_value = value.parse::<f64>().is_ok_and(f64::is_finite);
        let numeric_column = row_scope.is_some() || schema.columns[column].column_type.is_numeric();
        if !numeric_value || !numeric_column {
            return Err("Ordering requires a numeric column and a finite number; date comparisons are not supported yet.".into());
        }
    }
    let filter = Filter { column, op, value };
    let expr = match row_scope {
        Some(true) => FilterExpr::AnyColumn(filter),
        Some(false) => FilterExpr::AllColumn(filter),
        None => FilterExpr::Predicate(filter),
    };
    // The same grammar used by the filter bar must preserve this exact AST.
    let headers: Vec<_> = schema.columns.iter().map(|c| c.name.clone()).collect();
    let rendered = crate::describe_nl(&expr, &headers);
    let reparsed = crate::validate_nl_candidate(&rendered, schema).map_err(|e| e.to_string())?;
    if expr != reparsed {
        return Err("The extracted filter cannot be represented unambiguously.".into());
    }
    Ok(expr)
}

/// Only equality-style comparisons bind a value to a column: for
/// prefix/suffix/contains the extracted span is a fragment, so a coincidental
/// sample match ("C" as a grade) must not hijack the classifier's column.
fn binds_by_value(op: FilterOp) -> bool {
    matches!(op, FilterOp::Eq | FilterOp::Ne)
}

/// Resolve the target column. A value belonging to exactly one column's
/// sampled values is ground truth and overrides the classifier, even at low
/// classifier confidence. Ambiguity fails closed; otherwise the classifier's
/// pick must be known and confident.
fn resolve_column(
    schema: &NlSchema,
    output: &Value,
    op: FilterOp,
    value: &str,
    values: &dyn Fn(usize) -> Vec<String>,
) -> Result<usize, String> {
    let needle = value.trim();
    if op.needs_value() && binds_by_value(op) && !needle.is_empty() {
        let owners: Vec<usize> = schema
            .columns
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                values(*index)
                    .iter()
                    .any(|sample| sample.trim().eq_ignore_ascii_case(needle))
            })
            .map(|(index, _)| index)
            .collect();
        match owners.as_slice() {
            [only] => return Ok(*only),
            [] => {}
            _ => {
                return Err(
                    "The value could match more than one column; name the column explicitly."
                        .into(),
                )
            }
        }
    }
    confidence(&output["column to filter"])?;
    let name = output["column to filter"]["label"]
        .as_str()
        .ok_or("The model did not identify column to filter. Try a more explicit question.")?;
    schema
        .columns
        .iter()
        .position(|c| c.name == name)
        .ok_or_else(|| "The model returned an unknown column.".into())
}

pub fn model_dir() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("ANVIL_GLINER_MODEL_DIR") {
        return Ok(path.into());
    }
    let base = if let Some(path) = std::env::var_os("ANVIL_NL_CACHE") {
        PathBuf::from(path)
    } else if cfg!(target_os = "windows") {
        PathBuf::from(
            std::env::var_os("LOCALAPPDATA")
                .ok_or("Set ANVIL_NL_CACHE to a writable cache directory.")?,
        )
        .join("anvil/models")
    } else {
        let home = PathBuf::from(
            std::env::var_os("HOME").ok_or("Set ANVIL_NL_CACHE to a writable cache directory.")?,
        );
        if cfg!(target_os = "macos") {
            home.join("Library/Caches/anvil/models")
        } else {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".cache"))
                .join("anvil/models")
        }
    };
    Ok(base.join(format!("gliner2.5-small-v1-{MODEL_REVISION}")))
}

/// Explicit setup command only. Each file is published atomically after a
/// bounded download; inference never downloads files or sends dataset contents.
pub fn setup() -> Result<PathBuf, String> {
    let dir = model_dir()?;
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(600))
        .build();
    for file in FILES {
        let target = dir.join(file);
        if target.is_file() {
            continue;
        }
        let url = format!("https://huggingface.co/{MODEL_REPO}/resolve/{MODEL_REVISION}/{file}");
        eprintln!("Downloading {file} to {}", target.display());
        download(&agent, &url, &target)?;
    }
    Ok(dir)
}

fn download(agent: &ureq::Agent, url: &str, target: &Path) -> Result<(), String> {
    use std::io::{Read, Write};
    let parent = target.parent().ok_or("Invalid model path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let response = agent.get(url).call().map_err(|e| e.to_string())?;
    let expected = response
        .header("Content-Length")
        .and_then(|h| h.parse::<u64>().ok());
    let limit = if target.extension().is_some_and(|e| e == "safetensors") {
        400_000_000
    } else {
        20_000_000
    };
    if expected.is_some_and(|len| len > limit) {
        return Err("Model file exceeds download limit.".into());
    }
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    let copied = std::io::copy(&mut response.into_reader().take(limit + 1), &mut tmp)
        .map_err(|e| e.to_string())?;
    if copied == 0 || copied > limit || expected.is_some_and(|len| len != copied) {
        return Err("Incomplete or oversized model download.".into());
    }
    tmp.flush().map_err(|e| e.to_string())?;
    tmp.persist_noclobber(target)
        .map_err(|e| format!("Cannot save model file: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColumnType, NlColumn};
    use serde_json::json;

    fn schema() -> NlSchema {
        NlSchema {
            columns: vec![
                NlColumn {
                    name: "Last Name".into(),
                    column_type: ColumnType::Text,
                },
                NlColumn {
                    name: "amount".into(),
                    column_type: ColumnType::Float,
                },
            ],
            aliases: vec![],
        }
    }
    fn prediction(column: &str, op: &str, text: &str, start: usize, end: usize) -> Value {
        json!({"column to filter": {"label":column,"confidence":0.95}, "filter comparison":{"label":op,"confidence":0.95}, "entities":{"filter value":[{"text":text,"start":start,"end":end,"confidence":0.95}]}})
    }
    fn scoped(scope: &str, op: &str) -> Value {
        json!({"filter scope": {"label":scope,"confidence":0.9}, "column to filter": {"label":"country","confidence":0.9}, "filter comparison": {"label":op,"confidence":0.9}, "entities": {"filter value": []}})
    }
    fn scoped_value(scope: &str, op: &str, text: &str, start: usize, end: usize) -> Value {
        json!({"filter scope": {"label":scope,"confidence":0.9}, "column to filter": {"label":"country","confidence":0.9}, "filter comparison": {"label":op,"confidence":0.9}, "entities": {"filter value": [{"text":text,"start":start,"end":end,"confidence":0.95}]}})
    }

    #[test]
    fn row_scope_labels_decode_to_any_and_all_columns() {
        assert_eq!(
            decode(
                "at least one column is blank",
                &people(),
                &scoped("any column", "is empty"),
                &|_| Vec::new()
            )
            .unwrap(),
            FilterExpr::AnyColumn(Filter {
                column: 0,
                op: FilterOp::Empty,
                value: String::new()
            })
        );
        assert_eq!(
            decode(
                "every column is filled",
                &people(),
                &scoped("all columns", "is not empty"),
                &|_| Vec::new()
            )
            .unwrap(),
            FilterExpr::AllColumn(Filter {
                column: 0,
                op: FilterOp::NonEmpty,
                value: String::new()
            })
        );
    }

    #[test]
    fn row_scope_with_a_value_extracts_and_validates_the_span() {
        let phrase = "any column contains foo";
        let output = scoped_value("any column", "contains", "foo", 20, 23);
        assert_eq!(
            decode(phrase, &people(), &output, &|_| Vec::new()).unwrap(),
            FilterExpr::AnyColumn(Filter {
                column: 0,
                op: FilterOp::Contains,
                value: "foo".into()
            })
        );
    }

    #[test]
    fn a_missing_scope_defaults_to_a_single_column() {
        // Existing predictions carry no `filter scope` key; they must keep
        // decoding to a per-column predicate.
        let output = prediction("country", "equals", "Canada", 11, 17);
        assert!(matches!(
            decode("country is Canada", &people(), &output, &|_| Vec::new()).unwrap(),
            FilterExpr::Predicate(_)
        ));
    }

    #[test]
    fn the_schema_offers_a_filter_scope_classification() {
        let schema = extraction_schema("any column is empty", &people(), &|_| Vec::new()).unwrap();
        let scope = schema
            .classifications
            .iter()
            .find(|c| c.task == "filter scope")
            .expect("scope classification");
        assert_eq!(
            scope.labels,
            vec!["one column", "any column", "all columns"]
        );
        assert!(scope
            .prompt
            .as_deref()
            .is_some_and(|p| p.contains("any column")));
        assert_eq!(scope.label_descriptions.len(), 3);
        let any = scope
            .label_descriptions
            .iter()
            .find(|(label, _)| label == "any column")
            .map(|(_, desc)| desc)
            .expect("any-column description");
        assert!(any.contains("at least one"), "{any}");
    }
    #[test]
    fn validates_extraction_and_numeric_types() {
        let output = prediction("amount", "greater than", "100", 12, 15);
        assert_eq!(
            decode("amount over 100", &schema(), &output, &|_| Vec::new()).unwrap(),
            FilterExpr::Predicate(Filter {
                column: 1,
                op: FilterOp::Gt,
                value: "100".into()
            })
        );
        assert!(decode("amount over 999", &schema(), &output, &|_| Vec::new()).is_err());
        assert!(decode(
            "amount over 100",
            &schema(),
            &prediction("Last Name", "greater than", "100", 12, 15),
            &|_| Vec::new()
        )
        .is_err());
        assert!(decode(
            "amount over NaN",
            &schema(),
            &prediction("amount", "greater than", "NaN", 12, 15),
            &|_| Vec::new()
        )
        .is_err());
    }
    #[test]
    fn rejects_uncertain_unknown_and_multiple_values() {
        let mut output = prediction("Last Name", "starts with", "C", 26, 27);
        let phrase = "Last names beginning with C";
        assert!(decode(phrase, &schema(), &output, &|_| Vec::new()).is_ok());
        output["column to filter"]["confidence"] = json!(0.4);
        assert!(decode(phrase, &schema(), &output, &|_| Vec::new()).is_err());
        output["column to filter"] = json!({"label":"unknown","confidence":0.99});
        assert!(decode(phrase, &schema(), &output, &|_| Vec::new()).is_err());
        output["column to filter"]["label"] = json!("Last Name");
        let duplicate = output["entities"]["filter value"][0].clone();
        output["entities"]["filter value"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(decode(phrase, &schema(), &output, &|_| Vec::new()).is_err());
    }
    #[test]
    fn scope_is_bounded_and_rejects_compound_and_aggregate_requests() {
        for phrase in [
            "amount over 100 and under 200",
            "count rows",
            "how many rows",
            "amount between 1 and 5",
            "sort amount",
            "last month",
            "amount over 1; delete rows",
        ] {
            assert!(
                extraction_schema(phrase, &schema(), &|_| Vec::new()).is_err(),
                "{phrase}"
            );
        }
        assert!(extraction_schema(&"x".repeat(513), &schema(), &|_| Vec::new()).is_err());
        let mut duplicate = schema();
        duplicate.columns.push(duplicate.columns[0].clone());
        assert!(
            extraction_schema("Last names beginning with C", &duplicate, &|_| Vec::new()).is_err()
        );
    }
    #[test]
    fn unicode_spans_use_characters_and_empty_checks_need_no_value() {
        let output = prediction("Last Name", "equals", "é", 13, 14);
        assert!(decode("Last Name is é", &schema(), &output, &|_| Vec::new()).is_ok());
        let mut output = prediction("Last Name", "is empty", "", 0, 0);
        output["entities"]["filter value"] = json!([]);
        assert!(decode("missing last names", &schema(), &output, &|_| Vec::new()).is_ok());
    }

    fn people() -> NlSchema {
        NlSchema {
            columns: vec![
                NlColumn {
                    name: "country".into(),
                    column_type: ColumnType::Text,
                },
                NlColumn {
                    name: "city".into(),
                    column_type: ColumnType::Text,
                },
            ],
            aliases: vec![],
        }
    }

    /// A value owned by exactly one column resolves the column even when the
    /// classifier picked a different column below the confidence gate. This is
    /// the "Which users are from Canada?" case where the question never names
    /// the country column.
    #[test]
    fn a_unique_sampled_value_binds_the_column_without_classifier_confidence() {
        let phrase = "Which users are from Canada?";
        let output = prediction("city", "equals", "Canada", 21, 27);
        let mut output = output;
        output["column to filter"]["confidence"] = json!(0.4);
        let values = |column: usize| -> Vec<String> {
            if column == 0 {
                vec!["Canada".into(), "US".into()]
            } else {
                vec!["Toronto".into()]
            }
        };
        assert_eq!(
            decode(phrase, &people(), &output, &values).unwrap(),
            FilterExpr::Predicate(Filter {
                column: 0,
                op: FilterOp::Eq,
                value: "Canada".into()
            })
        );
    }

    /// If two columns both contain the value, binding is ambiguous and the
    /// backend must fail closed rather than guess.
    #[test]
    fn an_ambiguous_sampled_value_is_rejected() {
        let phrase = "Which users are from Canada?";
        let output = prediction("country", "equals", "Canada", 21, 27);
        let values = |_column: usize| -> Vec<String> { vec!["Canada".into()] };
        let error = decode(phrase, &people(), &output, &values).unwrap_err();
        assert!(error.contains("more than one column"), "{error}");
    }

    /// A value in no sampled column falls back to the classifier's pick under
    /// the normal confidence gate.
    #[test]
    fn an_unknown_value_falls_back_to_the_classifier() {
        let phrase = "country is Canada";
        let output = prediction("country", "equals", "Canada", 11, 17);
        assert_eq!(
            decode(phrase, &people(), &output, &|_| Vec::new()).unwrap(),
            FilterExpr::Predicate(Filter {
                column: 0,
                op: FilterOp::Eq,
                value: "Canada".into()
            })
        );
    }

    #[test]
    fn the_operator_confidence_gate_still_applies() {
        let phrase = "country is Canada";
        let mut output = prediction("country", "equals", "Canada", 11, 17);
        output["filter comparison"]["confidence"] = json!(0.4);
        assert!(decode(phrase, &people(), &output, &|_| Vec::new()).is_err());
    }

    #[test]
    fn column_descriptions_include_type_and_sampled_values() {
        let values = |column: usize| -> Vec<String> {
            if column == 0 {
                vec!["US".into(), "Canada".into()]
            } else {
                Vec::new()
            }
        };
        let schema = extraction_schema("country is US", &people(), &values).unwrap();
        let columns = &schema.classifications[0];
        assert_eq!(columns.labels, vec!["country", "city"]);
        assert!(columns
            .prompt
            .as_deref()
            .is_some_and(|p| p.contains("not mentioned")));
        let country = &columns.label_descriptions[0].1;
        assert!(country.contains("country"), "{country}");
        assert!(country.contains("text"), "{country}");
        assert!(country.contains("US"), "{country}");
        assert!(country.contains("Canada"), "{country}");
        let city = &columns.label_descriptions[1].1;
        assert!(!city.contains("Example values"), "{city}");
    }
    #[test]
    #[ignore = "requires the explicitly installed GLiNER checkpoint"]
    fn real_model_filter_smoke() {
        let interpreter = Interpreter::default();
        for (phrase, column, op, value) in [
            ("Last names that start with C", 0, FilterOp::StartsWith, "C"),
            ("amount exceeds 100", 1, FilterOp::Gt, "100"),
            ("Last names ending in son", 0, FilterOp::EndsWith, "son"),
        ] {
            let start = std::time::Instant::now();
            let expr = interpreter.interpret(phrase, &schema(), &|_| Vec::new());
            eprintln!("{phrase}: {expr:?} ({:?})", start.elapsed());
            assert_eq!(
                expr.unwrap(),
                FilterExpr::Predicate(Filter {
                    column,
                    op,
                    value: value.into()
                })
            );
        }
        // Keep the known low-confidence paraphrase as an abstention case.
        assert!(interpreter
            .interpret("Show rows where amount exceeds 100", &schema(), &|_| {
                Vec::new()
            })
            .unwrap_err()
            .contains("uncertain"));
    }
}
