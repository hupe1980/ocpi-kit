//! What the correctness guarantees cost.
//!
//! Two of this crate's decisions have a price: every `number` is a `Decimal` rather than an
//! `f64`, and every object carries a `#[serde(flatten)]` map so unknown fields survive. Both are
//! right, and both are slower than not doing them. These benchmarks say by how much, so the
//! answer is a measurement rather than an argument.
//!
//! ```console
//! cargo bench --features full
//! ```
//!
//! The corpus is the specification's own examples plus a synthetic page of 200 Locations, which
//! is the shape of a real `GET /locations` response.

use std::hint::black_box;
use std::path::Path;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use ocpi_kit::tariffs::{PricedPeriod, PricedSession, PricingEngine, TimeZone};
use ocpi_kit::transport::{OcpiResponse, Patch, merge};
use ocpi_kit::types::{DateTime, Number, Validate};
use ocpi_kit::v2_3_0::cdrs::Cdr;
use ocpi_kit::v2_3_0::locations::Location;
use ocpi_kit::v2_3_0::tariffs::Tariff;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/2.3.0").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// A page of Locations of the size a real Sender interface returns.
fn location_page(n: usize) -> Vec<Location> {
    (0..n).map(|i| ocpi_kit::testkit::sample::location(&format!("LOC{i}")).expect("valid sample")).collect()
}

// ---------------------------------------------------------------------------------------------
// Decode and encode
// ---------------------------------------------------------------------------------------------

fn decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode");

    macro_rules! decode_fixture {
        ($name:literal, $file:literal, $ty:ty) => {{
            let json = fixture($file);
            group.throughput(Throughput::Bytes(json.len() as u64));
            group.bench_function($name, |b| {
                b.iter(|| serde_json::from_str::<$ty>(black_box(&json)).unwrap());
            });
        }};
    }

    decode_fixture!("location", "location_example.json", Location);
    decode_fixture!("tariff", "tariff_4_complex.json", Tariff);

    // The spec's own `cdr_example.json` omits `tax_included` and does not decode — it is a
    // recorded erratum, see `tests/fixtures.rs` — so the CDR benchmark uses the testkit sample.
    let cdr_json = serde_json::to_string(&sample_cdr()).expect("serialises");
    group.throughput(Throughput::Bytes(cdr_json.len() as u64));
    group.bench_function("cdr", |b| {
        b.iter(|| serde_json::from_str::<Cdr>(black_box(&cdr_json)).unwrap());
    });

    group.finish();
}

fn sample_cdr() -> Cdr {
    ocpi_kit::testkit::sample::cdr("CDR1").expect("valid sample")
}

fn encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode");
    let location: Location = serde_json::from_str(&fixture("location_example.json")).unwrap();
    let cdr = sample_cdr();

    group.bench_function("location", |b| {
        b.iter(|| serde_json::to_string(black_box(&location)).unwrap());
    });
    group.bench_function("cdr", |b| {
        b.iter(|| serde_json::to_string(black_box(&cdr)).unwrap());
    });
    group.finish();
}

/// A whole page, which is what a crawl actually spends its time on.
fn pages(c: &mut Criterion) {
    let mut group = c.benchmark_group("page");
    for size in [10usize, 50, 200] {
        let page = location_page(size);
        let json = serde_json::to_string(&OcpiResponse::success(page.clone())).unwrap();
        group.throughput(Throughput::Bytes(json.len() as u64));

        group.bench_with_input(BenchmarkId::new("decode", size), &json, |b, json| {
            b.iter(|| serde_json::from_str::<OcpiResponse<Vec<Location>>>(black_box(json)).unwrap());
        });
        // `from_slice` avoids the UTF-8 validation `from_str` has already done for us; a real
        // client has bytes, not a `String`, so this is the number that matters to a crawl.
        let bytes = json.as_bytes().to_vec();
        group.bench_with_input(BenchmarkId::new("decode_slice", size), &bytes, |b, bytes| {
            b.iter(|| serde_json::from_slice::<OcpiResponse<Vec<Location>>>(black_box(bytes)).unwrap());
        });
        group.bench_with_input(BenchmarkId::new("validate", size), &page, |b, page| {
            b.iter(|| {
                for location in black_box(page) {
                    let _ = location.validate();
                }
            });
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------------------------
// The price of exact arithmetic
// ---------------------------------------------------------------------------------------------

fn numbers(c: &mut Criterion) {
    let mut group = c.benchmark_group("number");

    let decimals: Vec<Number> = (1..=1000).map(|i| Number::from(i) / Number::from(100u32)).collect();
    group.bench_function("sum_1000_decimals", |b| {
        b.iter(|| black_box(&decimals).iter().copied().sum::<Number>());
    });

    // The same sum in `f64`, for scale. It is faster, and it is also wrong: see the assertion.
    let floats: Vec<f64> = (1..=1000).map(|i| f64::from(i) / 100.0).collect();
    group.bench_function("sum_1000_floats", |b| {
        b.iter(|| black_box(&floats).iter().sum::<f64>());
    });

    let json = "0.2345";
    group.bench_function("parse", |b| {
        b.iter(|| serde_json::from_str::<Number>(black_box(json)).unwrap());
    });
    let n: Number = json.parse().unwrap();
    group.bench_function("write", |b| {
        b.iter(|| serde_json::to_string(black_box(&n)).unwrap());
    });

    group.finish();
}

// ---------------------------------------------------------------------------------------------
// The layers a hub spends its time in
// ---------------------------------------------------------------------------------------------

fn patching(c: &mut Criterion) {
    let mut group = c.benchmark_group("patch");
    let location: Location = serde_json::from_str(&fixture("location_example.json")).unwrap();
    let patch: Patch<Location> = Patch::from_value(serde_json::json!({
        "name": "Gent Zuid",
        "last_updated": "2024-03-01T10:00:00Z",
    }));

    group.bench_function("apply_to_location", |b| {
        b.iter(|| black_box(&patch).apply(black_box(&location)).unwrap());
    });

    let target = serde_json::to_value(&location).unwrap();
    let merge_patch = serde_json::json!({ "name": "Gent Zuid" });
    group.bench_function("merge_raw", |b| {
        b.iter_batched(
            || target.clone(),
            |mut t| {
                merge(&mut t, black_box(&merge_patch));
                t
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

#[cfg(feature = "convert")]
fn converting(c: &mut Criterion) {
    use ocpi_kit::convert::Downgrade;

    let mut group = c.benchmark_group("convert");
    let location: Location = serde_json::from_str(&fixture("location_example.json")).unwrap();
    group.bench_function("location_2_3_0_to_2_2_1", |b| {
        b.iter_batched(|| location.clone(), Downgrade::downgrade, BatchSize::SmallInput);
    });
    group.finish();
}

#[cfg(not(feature = "convert"))]
fn converting(_: &mut Criterion) {}

fn pricing(c: &mut Criterion) {
    let mut group = c.benchmark_group("pricing");
    let tariff: Tariff = serde_json::from_str(&fixture("tariff_4_complex.json")).unwrap();
    let tariffs = [tariff];

    let start: DateTime = "2024-01-15T10:00:00Z".parse().unwrap();
    let session =
        PricedSession::new(start, TimeZone::named("Europe/Berlin").unwrap()).with_period(PricedPeriod {
            energy_kwh: "20".parse().unwrap(),
            charging_hours: "1".parse().unwrap(),
            ..PricedPeriod::new(start)
        });

    group.bench_function("one_session", |b| {
        b.iter(|| PricingEngine::new().price(black_box(&session), black_box(&tariffs)).unwrap());
    });
    group.finish();
}

criterion_group!(benches, decode, encode, pages, numbers, patching, converting, pricing);
criterion_main!(benches);
