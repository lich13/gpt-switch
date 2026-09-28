use crate::usage::parser::Tokens;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, str::FromStr};
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rates {
    pub input: Option<String>,
    pub output: Option<String>,
    pub cache_read: Option<String>,
    pub cache_write: Option<String>,
    pub cache_hour: Option<String>,
    pub image_input: Option<String>,
    pub image_output: Option<String>,
    pub image: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variant {
    pub above: u64,
    pub tier: String,
    pub rates: Rates,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    pub model_id: String,
    pub display_name: String,
    pub source: String,
    pub fixed: bool,
    pub rates: Rates,
    #[serde(default)]
    pub variants: Vec<Variant>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteModel {
    pub key: String,
    pub provider: String,
    pub model_id: String,
    pub name: String,
    pub released: String,
    pub common: bool,
    pub price: Price,
}
pub fn decimal(s: &str) -> Option<Decimal> {
    Decimal::from_str(s)
        .or_else(|_| Decimal::from_scientific(s))
        .ok()
        .filter(|d| !d.is_sign_negative() && *d <= Decimal::from(1_000_000_000u64))
}
fn number(v: &Value, factor: u64) -> Option<String> {
    let raw = v
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string());
    decimal(&raw)?
        .checked_mul(Decimal::from(factor))
        .map(|d| d.normalize().to_string())
}
fn set_rate(rates: &mut Rates, key: &str, value: String) -> bool {
    let field = match key {
        "input_cost_per_token" => &mut rates.input,
        "output_cost_per_token" => &mut rates.output,
        "cache_read_input_token_cost" => &mut rates.cache_read,
        "cache_creation_input_token_cost" => &mut rates.cache_write,
        "cache_creation_input_token_cost_above_1hr" => &mut rates.cache_hour,
        "input_cost_per_image_token" | "input_cost_per_image_token_text" => &mut rates.image_input,
        "output_cost_per_image_token" => &mut rates.image_output,
        "output_cost_per_image" => &mut rates.image,
        _ => return false,
    };
    *field = Some(value);
    true
}
pub fn litellm(root: &Value) -> BTreeMap<String, Price> {
    let mut result = BTreeMap::new();
    let Some(models) = root.as_object() else {
        return result;
    };
    for (id, model) in models {
        if id == "sample_spec" || id.len() > 256 {
            continue;
        }
        let Some(fields) = model.as_object() else {
            continue;
        };
        let mut price = Price {
            model_id: id.clone(),
            display_name: id.clone(),
            source: "LiteLLM".into(),
            fixed: false,
            rates: Rates::default(),
            variants: vec![],
        };
        let mut variants: BTreeMap<(String, u64), Rates> = BTreeMap::new();
        for (raw_key, value) in fields {
            let mut key = raw_key.as_str();
            let tier = [
                ("_priority", "priority"),
                ("_flex", "flex"),
                ("_batches", "batch"),
            ]
            .into_iter()
            .find_map(|(suffix, tier)| {
                key.strip_suffix(suffix).map(|base| {
                    key = base;
                    tier
                })
            })
            .unwrap_or("default");
            let mut above = 0;
            let mut base = key.to_string();
            if let Some((prefix, suffix)) = key.split_once("_above_") {
                if let Some(n) = suffix
                    .strip_suffix("k_tokens")
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    above = n.saturating_mul(1000);
                    base = prefix.to_string();
                }
            }
            let factor = if base == "output_cost_per_image" {
                1
            } else {
                1_000_000
            };
            let Some(n) = number(value, factor) else {
                continue;
            };
            if tier == "default" && above == 0 {
                set_rate(&mut price.rates, &base, n);
            } else {
                set_rate(variants.entry((tier.into(), above)).or_default(), &base, n);
            }
        }
        price.variants = variants
            .into_iter()
            .map(|((tier, above), rates)| Variant { tier, above, rates })
            .collect();
        if price.rates.input.is_some()
            || price.rates.output.is_some()
            || price.rates.image.is_some()
        {
            result.insert(id.clone(), price);
        }
    }
    result
}
fn overlay(target: &mut Rates, source: &Rates) {
    macro_rules! fields { ($($f:ident),*) => { $(if source.$f.is_some() { target.$f=source.$f.clone(); })* }; }
    fields!(
        input,
        output,
        cache_read,
        cache_write,
        cache_hour,
        image_input,
        image_output,
        image
    );
}
impl Price {
    pub fn rates_for(&self, tier: Option<&str>, context: u64) -> Rates {
        let tier = match tier.unwrap_or("default") {
            "auto" => "default",
            "fast" => "priority",
            other => other,
        };
        let mut rates = self.rates.clone();
        let mut variants: Vec<_> = self
            .variants
            .iter()
            .filter(|v| v.tier == "default" && (v.above == 0 || context > v.above))
            .collect();
        variants.sort_by_key(|v| v.above);
        for v in variants {
            overlay(&mut rates, &v.rates);
        }
        if tier != "default" {
            let mut variants: Vec<_> = self
                .variants
                .iter()
                .filter(|v| v.tier == tier && (v.above == 0 || context > v.above))
                .collect();
            variants.sort_by_key(|v| v.above);
            for v in variants {
                overlay(&mut rates, &v.rates);
            }
        }
        rates
    }
    pub fn validate(&self) -> bool {
        !self.model_id.trim().is_empty()
            && self.model_id.len() <= 256
            && !self.model_id.chars().any(char::is_control)
            && self.display_name.len() <= 256
            && [&self.rates]
                .into_iter()
                .chain(self.variants.iter().map(|v| &v.rates))
                .all(|r| {
                    [
                        &r.input,
                        &r.output,
                        &r.cache_read,
                        &r.cache_write,
                        &r.cache_hour,
                        &r.image_input,
                        &r.image_output,
                        &r.image,
                    ]
                    .into_iter()
                    .all(|s| s.as_ref().is_none_or(|v| decimal(v).is_some()))
                })
    }
}
fn common_family<'a>(provider: &'a str, id: &str) -> Option<&'a str> {
    let prefixes: &[&str] = match provider {
        "anthropic" => &["claude-"],
        "openai" => &["gpt-", "o1-", "o3-", "o4-"],
        "google" => &["gemini-"],
        "xai" => &["grok-"],
        "deepseek" => &["deepseek-"],
        "alibaba" => &["qwen"],
        "xiaomi" => &["mimo-"],
        "longcat" => &["longcat-"],
        "moonshotai" => &["kimi-"],
        "minimax-cn" => &["minimax-m"],
        "zai" => &["glm-"],
        _ => return None,
    };
    prefixes
        .iter()
        .any(|p| id.to_ascii_lowercase().starts_with(p))
        .then_some(provider)
}
pub fn models_dev(root: &Value) -> Vec<RemoteModel> {
    let mut result = vec![];
    for (provider, p) in root.as_object().into_iter().flatten() {
        for (id, v) in p
            .get("models")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let cost = &v["cost"];
            let rates = Rates {
                input: number(&cost["input"], 1),
                output: number(&cost["output"], 1),
                cache_read: number(&cost["cache_read"], 1),
                cache_write: number(&cost["cache_write"], 1),
                ..Rates::default()
            };
            if rates.input.is_none() && rates.output.is_none() {
                continue;
            }
            let name = v["name"].as_str().unwrap_or(id).to_string();
            let mut variants = vec![];
            let parse_rates = |c: &Value| Rates {
                input: number(&c["input"], 1),
                output: number(&c["output"], 1),
                cache_read: number(&c["cache_read"], 1),
                cache_write: number(&c["cache_write"], 1),
                ..Rates::default()
            };
            if let Some(c) = cost.get("context_over_200k") {
                variants.push(Variant {
                    above: 200000,
                    tier: "default".into(),
                    rates: parse_rates(c),
                });
            }
            for c in cost["tiers"].as_array().into_iter().flatten() {
                if c.pointer("/tier/type").and_then(Value::as_str) == Some("context") {
                    if let Some(above) = c.pointer("/tier/size").and_then(Value::as_u64) {
                        variants.push(Variant {
                            above,
                            tier: "default".into(),
                            rates: parse_rates(c),
                        });
                    }
                }
            }
            let price = Price {
                model_id: id.clone(),
                display_name: name.clone(),
                source: format!("models.dev/{provider}"),
                fixed: false,
                rates,
                variants,
            };
            if !price.validate() {
                continue;
            }
            let text = v
                .pointer("/modalities/output")
                .and_then(Value::as_array)
                .is_none_or(|a| a.iter().all(|x| x == "text"));
            let lower = format!("{id} {name}").to_lowercase();
            let family = common_family(provider, id);
            let common = text
                && v["status"] != "deprecated"
                && ![
                    "embedding",
                    "image",
                    "audio",
                    "video",
                    "tts",
                    "moderation",
                    "transcribe",
                    "realtime",
                ]
                .iter()
                .any(|s| lower.contains(s))
                && family.is_some();
            result.push(RemoteModel {
                key: format!("{provider}::{id}"),
                provider: provider.clone(),
                model_id: id.clone(),
                name,
                released: v["release_date"].as_str().unwrap_or("").into(),
                common,
                price,
            });
        }
    }
    result.sort_by(|a, b| {
        b.released
            .cmp(&a.released)
            .then(a.name.cmp(&b.name))
            .then(a.key.cmp(&b.key))
    });
    let mut families: BTreeMap<String, usize> = BTreeMap::new();
    for m in &mut result {
        if m.common {
            let family = common_family(&m.provider, &m.model_id).unwrap_or("other");
            let count = families.entry(family.into()).or_default();
            *count += 1;
            m.common = *count <= 6;
        }
    }
    result
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    pub status: String,
    pub total: Option<String>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub cache_read: Option<String>,
    pub cache_write: Option<String>,
    pub image: Option<String>,
    pub pricing_model: Option<String>,
    pub price: Option<Price>,
    pub version: String,
}
pub fn calculate(
    tokens: &Tokens,
    price: Option<&Price>,
    tier: Option<&str>,
    multiplier: &str,
    version: &str,
) -> Cost {
    let mut cost = Cost {
        status: "unreported".into(),
        pricing_model: price.map(|p| p.model_id.clone()),
        price: price.cloned(),
        version: version.into(),
        ..Cost::default()
    };
    if !tokens.reported() {
        return cost;
    }
    let multiplier = decimal(multiplier).unwrap_or(Decimal::ONE);
    if multiplier.is_zero() {
        cost.status = "priced".into();
        cost.total = Some("0".into());
        return cost;
    }
    let Some(price) = price else {
        cost.status = "unpriced".into();
        return cost;
    };
    let r = price.rates_for(tier, tokens.context.unwrap_or(0));
    let mut missing = false;
    let mut add = |n: Option<u64>, rate: &Option<String>, per_million: bool| -> Option<Decimal> {
        let n = n?;
        if n == 0 {
            return Some(Decimal::ZERO);
        }
        let Some(rate) = rate.as_ref().and_then(|s| decimal(s)) else {
            missing = true;
            return None;
        };
        Decimal::from(n)
            .checked_mul(rate)?
            .checked_mul(multiplier)?
            .checked_div(Decimal::from(if per_million { 1_000_000 } else { 1 }))
    };
    let input_images = tokens
        .input_images
        .unwrap_or(0)
        .min(tokens.input.unwrap_or(0));
    let output_images = tokens
        .output_images
        .unwrap_or(0)
        .min(tokens.output.unwrap_or(0));
    let input = add(tokens.input.map(|n| n - input_images), &r.input, true);
    let output = add(tokens.output.map(|n| n - output_images), &r.output, true);
    let read = add(tokens.cache_read, &r.cache_read, true);
    let hour = tokens
        .cache_write_hour
        .unwrap_or(0)
        .min(tokens.cache_write.unwrap_or(0));
    let write = add(tokens.cache_write.map(|n| n - hour), &r.cache_write, true);
    let hour = add(tokens.cache_write_hour, &r.cache_hour, true);
    let image_in = add(
        tokens.input_images,
        &r.image_input.clone().or(r.input.clone()),
        true,
    );
    let image_out = add(
        tokens.output_images,
        &r.image_output.clone().or(r.output.clone()),
        true,
    );
    let image_count = if tokens.output_images.is_some() && r.image.is_none() {
        None
    } else {
        add(tokens.images, &r.image, false)
    };
    let values = [
        input,
        output,
        read,
        write,
        hour,
        image_in,
        image_out,
        image_count,
    ];
    let known = values.iter().any(Option::is_some);
    let fmt = |d: Option<Decimal>| d.map(|n| n.normalize().to_string());
    cost.input = fmt(input);
    cost.output = fmt(output);
    cost.cache_read = fmt(read);
    cost.cache_write = fmt(if write.is_some() || hour.is_some() {
        Some(write.unwrap_or_default() + hour.unwrap_or_default())
    } else {
        None
    });
    cost.image = fmt(
        if image_in.is_some() || image_out.is_some() || image_count.is_some() {
            Some(
                image_in.unwrap_or_default()
                    + image_out.unwrap_or_default()
                    + image_count.unwrap_or_default(),
            )
        } else {
            None
        },
    );
    cost.status = if missing {
        if known {
            "partial"
        } else {
            "unpriced"
        }
    } else {
        "priced"
    }
    .into();
    cost.total = known.then(|| {
        values
            .into_iter()
            .flatten()
            .sum::<Decimal>()
            .normalize()
            .to_string()
    });
    cost
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn units_context_priority_zero_and_missing() {
        let p = litellm(
            &serde_json::json!({"sample_spec":{"input_cost_per_token":1},"gpt-fixture":{
            "input_cost_per_token":0.000002,"output_cost_per_token":0.00001,"cache_read_input_token_cost":0.0000002,
            "input_cost_per_token_above_200k_tokens":0.000004,"input_cost_per_token_priority":0.000006}}),
        );
        assert_eq!(p.len(), 1);
        let p = &p["gpt-fixture"];
        assert_eq!(p.rates.input.as_deref(), Some("2"));
        let tokens = Tokens {
            input: Some(10),
            output: Some(2),
            cache_read: Some(90),
            context: Some(100),
            ..Tokens::default()
        };
        assert_eq!(
            calculate(&tokens, Some(p), None, "1", "v").total.as_deref(),
            Some("0.000058")
        );
        assert_eq!(p.rates_for(None, 200001).input.as_deref(), Some("4"));
        assert_eq!(
            p.rates_for(Some("priority"), 100).input.as_deref(),
            Some("6")
        );
        assert_eq!(calculate(&tokens, None, None, "1", "v").status, "unpriced");
        assert_eq!(
            calculate(&tokens, None, None, "0", "v").total.as_deref(),
            Some("0")
        );
    }
}
