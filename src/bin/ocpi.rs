//! `ocpi` — a command-line tool for working with OCPI peers and payloads.
//!
//! ```text
//! ocpi validate location.json --as location            # is this object conformant?
//! ocpi versions https://cpo.example.com/ocpi/versions --token …
//! ocpi pull locations https://… --token … --limit 50
//! ocpi pull payment-terminals https://… --token …
//! ocpi price cdr.json --tariff tariff.json --time-zone Europe/Berlin
//! ocpi convert location.json --from 2.2.1 --to 2.3.0   # with a loss report
//! ocpi schema location --version 2.3.0                  # JSON Schema
//! ```
//!
//! Everything the tool does is something the library does; the point is to make it available
//! without writing a program, so that a conformance question can be answered in one line.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use ocpi_kit::client::{Conformance, OcpiClient, Registration};
use ocpi_kit::tariffs::{PricedSession, PricingEngine, TimeZone};
use ocpi_kit::transport::{CredentialsToken, PageQuery, Quirks};
use ocpi_kit::types::{Url, Validate};
use ocpi_kit::{ModuleId, VersionNumber};

/// A toolkit for the OCPI protocol.
#[derive(Parser)]
#[command(name = "ocpi", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Allow plain HTTP and private network addresses.
    ///
    /// Off by default: `Credentials.url`, `Endpoint.url` and every `response_url` are supplied by
    /// a peer, and fetching them unconditionally makes this tool an SSRF proxy.
    #[arg(long, global = true)]
    insecure: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Check a JSON file against the specification.
    Validate {
        /// The file to check. `-` reads standard input.
        file: PathBuf,
        /// What the file is supposed to be.
        #[arg(long = "as", value_name = "OBJECT")]
        object: ObjectKind,
        /// Which OCPI version to check against.
        #[arg(long, default_value = "2.3.0")]
        version: SupportedVersion,
    },
    /// Fetch a peer's supported versions and the details of the newest common one.
    Versions {
        /// The peer's `/versions` URL.
        url: String,
        /// The credentials token to authenticate with.
        #[arg(long, env = "OCPI_TOKEN")]
        token: String,
        /// Send the token without Base64, for a 2.1.1 or 2.2 peer.
        #[arg(long)]
        unencoded_token: bool,
    },
    /// Check a live peer against the specification, without changing anything.
    Conformance {
        /// The peer's `/versions` URL.
        url: String,
        /// The credentials token to authenticate with — the `CREDENTIALS_TOKEN_C` of an existing
        /// registration.
        #[arg(long, env = "OCPI_TOKEN")]
        token: String,
        /// Send the token without Base64, for a 2.1.1 or 2.2 peer.
        #[arg(long)]
        unencoded_token: bool,
        /// How many objects to ask for per page.
        #[arg(long, default_value_t = 10)]
        limit: u64,
        /// Skip the two deliberately-rejected requests that check authentication.
        #[arg(long)]
        no_auth_checks: bool,
        /// Exit 0 even when the peer fails a check.
        #[arg(long)]
        no_fail: bool,
    },
    /// Crawl every page of a Sender list endpoint and print the objects.
    Pull {
        /// Which module to pull from.
        module: PullModule,
        /// The peer's `/versions` URL.
        url: String,
        /// The credentials token to authenticate with.
        #[arg(long, env = "OCPI_TOKEN")]
        token: String,
        /// The party to send `OCPI-from-*` as, written `NL/TNM`.
        #[arg(long, default_value = "NL/TNM")]
        from: String,
        /// Only objects updated at or after this time.
        #[arg(long)]
        since: Option<String>,
        /// Page size to request.
        #[arg(long)]
        limit: Option<u64>,
        /// Stop after this many objects.
        #[arg(long)]
        max: Option<usize>,
    },
    /// Compute what a CDR should have cost, and show the breakdown.
    Price {
        /// A CDR, as JSON.
        cdr: PathBuf,
        /// The Tariffs to price against; may be given several times.
        #[arg(long = "tariff", required = true)]
        tariffs: Vec<PathBuf>,
        /// The IANA time zone of the Location, which the local-time restrictions use.
        #[arg(long, default_value = "UTC")]
        time_zone: String,
        /// Bill measured quantities exactly, ignoring `step_size`, as OCPI 3.0 will.
        #[arg(long)]
        no_step_size: bool,
        /// Print the breakdown as JSON rather than as a table.
        #[arg(long)]
        json: bool,
    },
    /// Convert an object between OCPI versions, reporting what did not survive.
    Convert {
        /// The file to convert.
        file: PathBuf,
        /// What the file is.
        #[arg(long = "as", value_name = "OBJECT")]
        object: ConvertibleKind,
        /// The version the file is written in.
        #[arg(long)]
        from: SupportedVersion,
        /// The version to convert it to.
        #[arg(long)]
        to: SupportedVersion,
    },
    /// Serve a conformant OCPI party in memory, for a partner to integrate against.
    ServeMock {
        /// Address to listen on.
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: String,
        /// The base URL the endpoints are published under, as a partner will reach them.
        ///
        /// Defaults to `http://<bind>`, which is right for local integration and wrong the moment
        /// there is a reverse proxy in front — the version details are generated from it.
        #[arg(long)]
        base_url: Option<String>,
        /// Which role the mock fills, which decides the party it speaks as.
        #[arg(long, value_enum, default_value = "cpo")]
        role: MockRole,
        /// Which OCPI version to publish. A 2.2.1 mock answers 2.2.1 bytes.
        #[arg(long, default_value = "2.3.0")]
        version: MockVersion,
        /// Start with no objects at all, rather than one of each.
        #[arg(long)]
        empty: bool,
    },
    /// Print the JSON Schema of an OCPI object.
    Schema {
        /// Which object.
        object: ObjectKind,
        /// Which OCPI version.
        #[arg(long, default_value = "2.3.0")]
        version: SupportedVersion,
    },
}

/// The role a `serve-mock` party fills.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum MockRole {
    /// A Charge Point Operator: it owns Locations, Sessions, CDRs and Tariffs.
    Cpo,
    /// An e-Mobility Service Provider: it owns Tokens.
    Emsp,
}

/// The versions `serve-mock` can publish — the ones this build can write on the wire.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum MockVersion {
    #[value(name = "2.2.1")]
    V2_2_1,
    #[value(name = "2.3.0")]
    V2_3_0,
}

impl From<MockVersion> for VersionNumber {
    fn from(value: MockVersion) -> Self {
        match value {
            MockVersion::V2_2_1 => Self::V2_2_1,
            MockVersion::V2_3_0 => Self::V2_3_0,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SupportedVersion {
    #[value(name = "2.1.1")]
    V2_1_1,
    #[value(name = "2.2.1")]
    V2_2_1,
    #[value(name = "2.3.0")]
    V2_3_0,
}

impl From<SupportedVersion> for VersionNumber {
    fn from(value: SupportedVersion) -> Self {
        match value {
            SupportedVersion::V2_1_1 => Self::V2_1_1,
            SupportedVersion::V2_2_1 => Self::V2_2_1,
            SupportedVersion::V2_3_0 => Self::V2_3_0,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ObjectKind {
    Location,
    Evse,
    Connector,
    Session,
    Cdr,
    Tariff,
    Token,
    Credentials,
    VersionDetails,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ConvertibleKind {
    Location,
    Session,
    Cdr,
    Tariff,
    Token,
    Credentials,
    Price,
}

/// Every Sender interface that answers with a paginated list.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum PullModule {
    Locations,
    Sessions,
    Cdrs,
    Tariffs,
    Tokens,
    HubClientInfo,
    /// The Payments module's terminals list.
    PaymentTerminals,
    /// The Payments module's financial advice confirmations list.
    FinancialAdviceConfirmations,
}

impl PullModule {
    const fn module(self) -> ModuleId {
        match self {
            Self::Locations => ModuleId::Locations,
            Self::Sessions => ModuleId::Sessions,
            Self::Cdrs => ModuleId::Cdrs,
            Self::Tariffs => ModuleId::Tariffs,
            Self::Tokens => ModuleId::Tokens,
            Self::HubClientInfo => ModuleId::HubClientInfo,
            Self::PaymentTerminals | Self::FinancialAdviceConfirmations => ModuleId::Payments,
        }
    }

    /// The sub-path below the discovered endpoint, for the one module that has two of them.
    ///
    /// Payments declares a single `ModuleID` and then addresses its two interfaces through two
    /// different endpoint variables, which version discovery cannot express; the discovered
    /// `payments` endpoint is therefore the base these hang off. See
    /// `SenderEndpoint::payments_terminals`.
    const fn sub_path(self) -> Option<&'static str> {
        match self {
            Self::PaymentTerminals => Some("terminals"),
            Self::FinancialAdviceConfirmations => Some("financial-advice-confirmations"),
            _ => None,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

type Fallible = Result<(), Box<dyn std::error::Error>>;

fn run(cli: Cli) -> Fallible {
    match cli.command {
        Command::Validate { file, object, version } => validate(&file, object, version),
        Command::Price { cdr, tariffs, time_zone, no_step_size, json } => {
            price(&cdr, &tariffs, &time_zone, no_step_size, json)
        }
        Command::Convert { file, object, from, to } => convert(&file, object, from, to),
        Command::Schema { object, version } => schema(object, version),
        Command::Versions { url, token, unencoded_token } => {
            block_on(versions(&url, &token, unencoded_token, cli.insecure))
        }
        Command::Conformance { url, token, unencoded_token, limit, no_auth_checks, no_fail } => block_on(
            conformance(&url, &token, unencoded_token, limit, !no_auth_checks, no_fail, cli.insecure),
        ),
        Command::Pull { module, url, token, from, since, limit, max } => {
            block_on(pull(module, &url, &token, &from, since.as_deref(), limit, max, cli.insecure))
        }
        Command::ServeMock { bind, base_url, role, version, empty } => {
            block_on(serve_mock(&bind, base_url.as_deref(), role, version.into(), !empty))
        }
    }
}

fn block_on<F: core::future::Future<Output = Fallible>>(future: F) -> Fallible {
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(future)
}

fn read(path: &std::path::Path) -> Result<String, Box<dyn std::error::Error>> {
    if path.as_os_str() == "-" {
        use std::io::Read as _;
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer)?;
        return Ok(buffer);
    }
    Ok(std::fs::read_to_string(path)?)
}

/// Decodes and validates one object, printing every violation with its JSON Pointer.
macro_rules! check {
    ($json:expr, $ty:ty) => {{
        let value: $ty = serde_json::from_str($json)?;
        report(value.validate())
    }};
}

fn validate(path: &std::path::Path, object: ObjectKind, version: SupportedVersion) -> Fallible {
    use ocpi_kit::{v2_1_1, v2_2_1, v2_3_0};
    let json = read(path)?;
    let ok = match (version, object) {
        (SupportedVersion::V2_3_0, ObjectKind::Location) => check!(&json, v2_3_0::locations::Location),
        (SupportedVersion::V2_3_0, ObjectKind::Evse) => check!(&json, v2_3_0::locations::Evse),
        (SupportedVersion::V2_3_0, ObjectKind::Connector) => {
            check!(&json, v2_3_0::locations::Connector)
        }
        (SupportedVersion::V2_3_0, ObjectKind::Session) => check!(&json, v2_3_0::sessions::Session),
        (SupportedVersion::V2_3_0, ObjectKind::Cdr) => check!(&json, v2_3_0::cdrs::Cdr),
        (SupportedVersion::V2_3_0, ObjectKind::Tariff) => check!(&json, v2_3_0::tariffs::Tariff),
        (SupportedVersion::V2_3_0, ObjectKind::Token) => check!(&json, v2_3_0::tokens::Token),
        (SupportedVersion::V2_3_0, ObjectKind::Credentials) => {
            check!(&json, v2_3_0::credentials::Credentials)
        }
        (SupportedVersion::V2_3_0, ObjectKind::VersionDetails) => {
            check!(&json, v2_3_0::versions::VersionDetails)
        }

        (SupportedVersion::V2_2_1, ObjectKind::Location) => check!(&json, v2_2_1::locations::Location),
        (SupportedVersion::V2_2_1, ObjectKind::Evse) => check!(&json, v2_2_1::locations::Evse),
        (SupportedVersion::V2_2_1, ObjectKind::Connector) => {
            check!(&json, v2_2_1::locations::Connector)
        }
        (SupportedVersion::V2_2_1, ObjectKind::Session) => check!(&json, v2_2_1::sessions::Session),
        (SupportedVersion::V2_2_1, ObjectKind::Cdr) => check!(&json, v2_2_1::cdrs::Cdr),
        (SupportedVersion::V2_2_1, ObjectKind::Tariff) => check!(&json, v2_2_1::tariffs::Tariff),
        (SupportedVersion::V2_2_1, ObjectKind::Token) => check!(&json, v2_2_1::tokens::Token),
        (SupportedVersion::V2_2_1, ObjectKind::Credentials) => {
            check!(&json, v2_2_1::credentials::Credentials)
        }
        (SupportedVersion::V2_2_1, ObjectKind::VersionDetails) => {
            check!(&json, v2_2_1::versions::VersionDetails)
        }

        (SupportedVersion::V2_1_1, ObjectKind::Location) => check!(&json, v2_1_1::locations::Location),
        (SupportedVersion::V2_1_1, ObjectKind::Evse) => check!(&json, v2_1_1::locations::Evse),
        (SupportedVersion::V2_1_1, ObjectKind::Connector) => {
            check!(&json, v2_1_1::locations::Connector)
        }
        (SupportedVersion::V2_1_1, ObjectKind::Session) => check!(&json, v2_1_1::sessions::Session),
        (SupportedVersion::V2_1_1, ObjectKind::Cdr) => check!(&json, v2_1_1::cdrs::Cdr),
        (SupportedVersion::V2_1_1, ObjectKind::Tariff) => check!(&json, v2_1_1::tariffs::Tariff),
        (SupportedVersion::V2_1_1, ObjectKind::Token) => check!(&json, v2_1_1::tokens::Token),
        (SupportedVersion::V2_1_1, ObjectKind::Credentials) => {
            check!(&json, v2_1_1::credentials::Credentials)
        }
        (SupportedVersion::V2_1_1, ObjectKind::VersionDetails) => {
            check!(&json, v2_1_1::versions::VersionDetails)
        }
    };
    if ok { Ok(()) } else { Err("the object does not conform to the specification".into()) }
}

fn report(result: Result<(), ocpi_kit::types::Violations>) -> bool {
    match result {
        Ok(()) => {
            println!("conformant");
            true
        }
        Err(violations) => {
            for violation in &violations {
                println!("{}\t{}\t{}", violation.code.as_str(), violation.pointer, violation.message);
            }
            false
        }
    }
}

fn price(
    cdr_path: &std::path::Path,
    tariff_paths: &[PathBuf],
    time_zone: &str,
    no_step_size: bool,
    as_json: bool,
) -> Fallible {
    let cdr: ocpi_kit::v2_3_0::cdrs::Cdr = serde_json::from_str(&read(cdr_path)?)?;
    let mut tariffs = Vec::with_capacity(tariff_paths.len());
    for path in tariff_paths {
        tariffs.push(serde_json::from_str::<ocpi_kit::v2_3_0::tariffs::Tariff>(&read(path)?)?);
    }

    let zone = TimeZone::named(time_zone)?;
    let session = PricedSession::from_cdr(&cdr, zone);
    let policy = if no_step_size {
        ocpi_kit::tariffs::PricingPolicy::default().without_step_size()
    } else {
        ocpi_kit::tariffs::PricingPolicy::default()
    };
    let breakdown = PricingEngine::with_policy(policy).price(&session, &tariffs)?;

    let claimed = cdr.total_cost.before_taxes;
    let agrees = claimed == breakdown.total_excl_vat;

    if as_json {
        println!("{}", serde_json::to_string_pretty(&breakdown)?);
    } else {
        println!("{breakdown}");
        if agrees {
            println!("\nthe CDR's own total agrees");
        } else {
            println!(
                "\nthe CDR claims {claimed} excl. tax, which differs by {}",
                claimed - breakdown.total_excl_vat
            );
        }
    }

    // Exit non-zero when the invoice does not check out, so this can be a pipeline step rather
    // than something a person has to read. A note is enough on its own: a CDR whose Charging
    // Periods span a price change can total correctly by luck and still be malformed.
    if !agrees || breakdown.needs_review() {
        return Err(Box::new(PricingDisagreement));
    }
    Ok(())
}

/// The CDR did not price to what it claims, or priced with findings.
#[derive(Debug)]
struct PricingDisagreement;

impl std::fmt::Display for PricingDisagreement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the CDR did not reconcile; see the breakdown above")
    }
}

impl std::error::Error for PricingDisagreement {}

fn convert(
    path: &std::path::Path,
    object: ConvertibleKind,
    from: SupportedVersion,
    to: SupportedVersion,
) -> Fallible {
    use ocpi_kit::convert::{Converted, Downgrade, Upgrade};
    use ocpi_kit::{v2_2_1, v2_3_0};

    /// Prints the converted object and its loss report.
    fn emit<T: serde::Serialize>(converted: Converted<T>) -> Fallible {
        println!("{}", serde_json::to_string_pretty(&converted.value)?);
        for loss in converted.lossy {
            eprintln!("lost\t{}\t{}", loss.pointer, loss.reason);
        }
        Ok(())
    }

    let json = read(path)?;

    match (from, to, object) {
        (SupportedVersion::V2_2_1, SupportedVersion::V2_3_0, kind) => match kind {
            ConvertibleKind::Location => {
                emit(serde_json::from_str::<v2_2_1::locations::Location>(&json)?.upgrade())
            }
            ConvertibleKind::Session => {
                emit(serde_json::from_str::<v2_2_1::sessions::Session>(&json)?.upgrade())
            }
            ConvertibleKind::Cdr => emit(serde_json::from_str::<v2_2_1::cdrs::Cdr>(&json)?.upgrade()),
            ConvertibleKind::Tariff => {
                emit(serde_json::from_str::<v2_2_1::tariffs::Tariff>(&json)?.upgrade())
            }
            ConvertibleKind::Token => emit(serde_json::from_str::<v2_2_1::tokens::Token>(&json)?.upgrade()),
            ConvertibleKind::Credentials => {
                emit(serde_json::from_str::<v2_2_1::credentials::Credentials>(&json)?.upgrade())
            }
            ConvertibleKind::Price => emit(serde_json::from_str::<v2_2_1::types::Price>(&json)?.upgrade()),
        },
        (SupportedVersion::V2_3_0, SupportedVersion::V2_2_1, kind) => match kind {
            ConvertibleKind::Location => {
                emit(serde_json::from_str::<v2_3_0::locations::Location>(&json)?.downgrade())
            }
            ConvertibleKind::Session => {
                emit(serde_json::from_str::<v2_3_0::sessions::Session>(&json)?.downgrade())
            }
            ConvertibleKind::Cdr => emit(serde_json::from_str::<v2_3_0::cdrs::Cdr>(&json)?.downgrade()),
            ConvertibleKind::Tariff => {
                emit(serde_json::from_str::<v2_3_0::tariffs::Tariff>(&json)?.downgrade())
            }
            ConvertibleKind::Token => emit(serde_json::from_str::<v2_3_0::tokens::Token>(&json)?.downgrade()),
            ConvertibleKind::Credentials => {
                emit(serde_json::from_str::<v2_3_0::credentials::Credentials>(&json)?.downgrade())
            }
            ConvertibleKind::Price => emit(serde_json::from_str::<v2_3_0::types::Price>(&json)?.downgrade()),
        },
        (a, b, _) => Err(format!(
            "no conversion between OCPI {} and {} is implemented; \
             2.2.1 ↔ 2.3.0 is the bridge a hub needs today",
            VersionNumber::from(a),
            VersionNumber::from(b)
        )
        .into()),
    }
}

fn schema(object: ObjectKind, version: SupportedVersion) -> Fallible {
    use ocpi_kit::{v2_1_1, v2_2_1, v2_3_0};

    macro_rules! emit {
        ($ty:ty) => {{
            let schema = schemars::schema_for!($ty);
            println!("{}", serde_json::to_string_pretty(&schema)?);
            return Ok(());
        }};
    }

    match (version, object) {
        (SupportedVersion::V2_3_0, ObjectKind::Location) => emit!(v2_3_0::locations::Location),
        (SupportedVersion::V2_3_0, ObjectKind::Evse) => emit!(v2_3_0::locations::Evse),
        (SupportedVersion::V2_3_0, ObjectKind::Connector) => emit!(v2_3_0::locations::Connector),
        (SupportedVersion::V2_3_0, ObjectKind::Session) => emit!(v2_3_0::sessions::Session),
        (SupportedVersion::V2_3_0, ObjectKind::Cdr) => emit!(v2_3_0::cdrs::Cdr),
        (SupportedVersion::V2_3_0, ObjectKind::Tariff) => emit!(v2_3_0::tariffs::Tariff),
        (SupportedVersion::V2_3_0, ObjectKind::Token) => emit!(v2_3_0::tokens::Token),
        (SupportedVersion::V2_3_0, ObjectKind::Credentials) => emit!(v2_3_0::credentials::Credentials),
        (SupportedVersion::V2_3_0, ObjectKind::VersionDetails) => {
            emit!(v2_3_0::versions::VersionDetails)
        }
        (SupportedVersion::V2_2_1, ObjectKind::Location) => emit!(v2_2_1::locations::Location),
        (SupportedVersion::V2_2_1, ObjectKind::Evse) => emit!(v2_2_1::locations::Evse),
        (SupportedVersion::V2_2_1, ObjectKind::Connector) => emit!(v2_2_1::locations::Connector),
        (SupportedVersion::V2_2_1, ObjectKind::Session) => emit!(v2_2_1::sessions::Session),
        (SupportedVersion::V2_2_1, ObjectKind::Cdr) => emit!(v2_2_1::cdrs::Cdr),
        (SupportedVersion::V2_2_1, ObjectKind::Tariff) => emit!(v2_2_1::tariffs::Tariff),
        (SupportedVersion::V2_2_1, ObjectKind::Token) => emit!(v2_2_1::tokens::Token),
        (SupportedVersion::V2_2_1, ObjectKind::Credentials) => emit!(v2_2_1::credentials::Credentials),
        (SupportedVersion::V2_2_1, ObjectKind::VersionDetails) => {
            emit!(v2_2_1::versions::VersionDetails)
        }
        (SupportedVersion::V2_1_1, ObjectKind::Location) => emit!(v2_1_1::locations::Location),
        (SupportedVersion::V2_1_1, ObjectKind::Evse) => emit!(v2_1_1::locations::Evse),
        (SupportedVersion::V2_1_1, ObjectKind::Connector) => emit!(v2_1_1::locations::Connector),
        (SupportedVersion::V2_1_1, ObjectKind::Session) => emit!(v2_1_1::sessions::Session),
        (SupportedVersion::V2_1_1, ObjectKind::Cdr) => emit!(v2_1_1::cdrs::Cdr),
        (SupportedVersion::V2_1_1, ObjectKind::Tariff) => emit!(v2_1_1::tariffs::Tariff),
        (SupportedVersion::V2_1_1, ObjectKind::Token) => emit!(v2_1_1::tokens::Token),
        (SupportedVersion::V2_1_1, ObjectKind::Credentials) => emit!(v2_1_1::credentials::Credentials),
        (SupportedVersion::V2_1_1, ObjectKind::VersionDetails) => {
            emit!(v2_1_1::versions::VersionDetails)
        }
    }
}

fn client_for(insecure: bool) -> Result<OcpiClient, Box<dyn std::error::Error>> {
    let config = if insecure {
        ocpi_kit::client::ClientConfig::for_testing()
    } else {
        ocpi_kit::client::ClientConfig::default()
    };
    Ok(OcpiClient::with_config(config)?)
}

async fn versions(url: &str, token: &str, unencoded: bool, insecure: bool) -> Fallible {
    let client = client_for(insecure)?;
    let mut registration = Registration::new(Url::new(url)?, CredentialsToken::new(token)?);
    if unencoded {
        let mut quirks = Quirks::default();
        quirks.send_unencoded_token = true;
        quirks.accept_unencoded_token = true;
        registration = registration.with_quirks(quirks);
    }

    let discovered = registration.discover(client.transport()).await?;
    println!("versions the peer offers:");
    for version in discovered.versions() {
        let mark = if version.version.is_supported() { "*" } else { " " };
        println!("  {mark} {:<6} {}", version.version.as_str(), version.url);
    }
    println!("  (* = this build can speak it)");

    let selected = discovered.select_best(client.transport()).await?;
    println!("\nendpoints for OCPI {}:", selected.version());
    for endpoint in &selected.details().endpoints {
        println!("  {:<22} {:<8} {}", endpoint.identifier.as_str(), endpoint.role, endpoint.url);
    }
    if let Err(violations) = selected.details().validate() {
        println!("\nthe version details do not fully conform:");
        for violation in &violations {
            println!("  {}\t{}", violation.pointer, violation.message);
        }
    }
    Ok(())
}

#[allow(clippy::fn_params_excessive_bools)]
async fn conformance(
    url: &str,
    token: &str,
    unencoded: bool,
    limit: u64,
    auth_checks: bool,
    no_fail: bool,
    insecure: bool,
) -> Fallible {
    let client = client_for(insecure)?;
    let mut run = Conformance::new(Url::new(url)?, CredentialsToken::new(token)?)
        .with_page_limit(limit)
        .with_auth_checks(auth_checks);
    if unencoded {
        let mut quirks = Quirks::default();
        quirks.send_unencoded_token = true;
        quirks.accept_unencoded_token = true;
        run = run.with_quirks(quirks);
    }

    let report = run.run(client.transport()).await;
    print!("{report}");
    if let Some(version) = &report.version {
        println!("checked against OCPI {version}");
    }

    if report.has_failures() && !no_fail {
        return Err(format!(
            "{} check(s) contradict the specification; pass --no-fail to exit 0 anyway",
            report.failures().count()
        )
        .into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn pull(
    module: PullModule,
    url: &str,
    token: &str,
    from: &str,
    since: Option<&str>,
    limit: Option<u64>,
    max: Option<usize>,
    insecure: bool,
) -> Fallible {
    let client = client_for(insecure)?;
    let details = Registration::new(Url::new(url)?, CredentialsToken::new(token)?)
        .discover(client.transport())
        .await?
        .select_best(client.transport())
        .await?
        // The peer already registered us out of band; reuse the token for the pull.
        .details()
        .clone();

    // The version that was negotiated, not the one this build prefers: it decides the
    // interoperability quirks and, for a typed pull, the translation into the canonical model.
    let peer = ocpi_kit::client::Peer::builder(details.version.clone(), CredentialsToken::new(token)?)
        .endpoints_from(&details)
        .build();

    let mut query = PageQuery::new();
    if let Some(since) = since {
        query = PageQuery::since(since.parse()?);
    }
    if let Some(limit) = limit {
        query = query.with_limit(limit);
    }

    let from: ocpi_kit::types::PartyRef = from.parse()?;
    let client_module = peer.module(client.transport(), module.module(), from);
    let mut stream = match module.sub_path() {
        None => client_module.list::<serde_json::Value>(query)?,
        Some(segment) => {
            let endpoint = client_module
                .sender_endpoint()
                .ok_or("the peer does not implement the Sender interface of this module")?;
            client_module.list_from::<serde_json::Value>(&endpoint.base().join(segment), &query)
        }
    };
    let mut count = 0usize;
    while let Some(object) = stream.next().await? {
        println!("{}", serde_json::to_string(&object)?);
        count += 1;
        if max.is_some_and(|m| count >= m) {
            break;
        }
    }
    eprintln!(
        "{count} object(s) over {} page(s), {} pagination correction(s)",
        stream.pages_fetched(),
        stream.corrections()
    );
    Ok(())
}

/// Serves a conformant OCPI party out of memory.
///
/// The point is the *other* side of an integration: a partner writing a client has nothing to
/// point it at until you are ready, and what they need is not a fixture file but an endpoint that
/// paginates, filters, refuses a write under the wrong party and answers `2004` for a token it
/// does not know. That is [`MockPeer`](ocpi_kit::testkit::MockPeer), and this is it on a socket.
async fn serve_mock(
    bind: &str,
    base_url: Option<&str>,
    role: MockRole,
    version: VersionNumber,
    seeded: bool,
) -> Fallible {
    use ocpi_kit::server::OcpiRouter;
    use ocpi_kit::testkit::MockPeer;

    let listener = tokio::net::TcpListener::bind(bind).await?;
    let address = listener.local_addr()?;
    // The base URL is what the generated version details publish, so it has to be what a partner
    // can actually reach — which is not necessarily what we bound to.
    let base = Url::new_lenient(base_url.map_or_else(|| format!("http://{address}"), ToOwned::to_owned));

    let peer = match role {
        MockRole::Cpo => MockPeer::cpo(base.clone()),
        MockRole::Emsp => MockPeer::msp(base.clone()),
    };
    let peer = if seeded { peer.seeded() } else { peer };
    let app = peer.mount(OcpiRouter::new(version.clone(), base.clone(), peer.token_store())).build();

    eprintln!("ocpi-kit mock {role:?} speaking OCPI {version} as {}", peer.party());
    eprintln!("  listening on   http://{address}");
    eprintln!("  published as   {base}");
    eprintln!("  versions       {}", base.join("versions"));
    eprintln!("  token          {} (also -a, as CREDENTIALS_TOKEN_A)", ocpi_kit::testkit::test_token("c"));
    eprintln!();
    eprintln!("  ocpi conformance {} --token {}", base.join("versions"), TOKEN_C);
    axum::serve(listener, app).await?;
    Ok(())
}

/// The mock's credentials token, in the clear: it is a published constant, not a secret.
///
/// `CredentialsToken` redacts itself in `Display` — which is right everywhere else and unhelpful
/// in the one line whose job is to tell a partner what to send.
const TOKEN_C: &str = "test-token-c";
