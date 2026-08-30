//! One trait per module and interface. Implement the ones your role serves; the router mounts
//! exactly those and generates version details that say so.
//!
//! Every method takes a [`RequestContext`], which carries the authenticated platform, the request
//! IDs and the routing headers. Persistence is deliberately absent: OCPI says nothing about how a
//! party stores its objects, and neither does this crate.
//!
//! # Why the traits use `impl Future` rather than `async fn`
//!
//! A trait method declared `async fn` produces a future with no `Send` bound, which an axum
//! handler cannot spawn. Declaring `-> impl Future<Output = …> + Send` fixes the bound while
//! still letting an implementation write a plain `async fn`.

use core::future::Future;

use crate::transport::{OcpiError, Page, PageQuery, Patch};
use crate::types::PartyRef;

use super::extract::RequestContext;

/// The result every handler method returns.
pub type Handled<T> = Result<T, OcpiError>;

// ---------------------------------------------------------------------------------------------
// Locations
// ---------------------------------------------------------------------------------------------

/// The CPO side of the Locations module: serving the Locations this party owns.
///
/// Spec: 2.3.0 §mod_locations_cpo_interface
pub trait LocationsSender: Send + Sync + 'static {
    /// `GET {locations}` — one page of Locations.
    ///
    /// The returned [`Page`] carries the `Link`, `X-Total-Count` and `X-Limit` values; build it
    /// with [`Page::single`] when everything fits in one response.
    fn list(
        &self,
        query: PageQuery,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Page<crate::v2_3_0::locations::Location>>> + Send;

    /// `GET {locations}/{location_id}`.
    fn location(
        &self,
        location_id: String,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::locations::Location>> + Send;

    /// `GET {locations}/{location_id}/{evse_uid}`.
    fn evse(
        &self,
        location_id: String,
        evse_uid: String,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::locations::Evse>> + Send;

    /// `GET {locations}/{location_id}/{evse_uid}/{connector_id}`.
    fn connector(
        &self,
        location_id: String,
        evse_uid: String,
        connector_id: String,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::locations::Connector>> + Send;
}

/// The eMSP side of the Locations module: receiving the Locations another party owns.
///
/// These are client-owned objects, so every method carries the `owner` from the URL. The router
/// has already checked that the authenticated platform speaks for that party.
///
/// > *An EVSE is never deleted; a removed EVSE gets `status` `REMOVED`* — so there is no delete
/// > method here, by design.
///
/// Spec: 2.3.0 §mod_locations_emsp_interface
pub trait LocationsReceiver: Send + Sync + 'static {
    /// `GET {locations}/{cc}/{party}/{location_id}` — what this party has stored.
    fn location(
        &self,
        owner: PartyRef,
        location_id: String,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::locations::Location>> + Send;

    /// `PUT {locations}/{cc}/{party}/{location_id}`.
    ///
    /// Returns whether the object was newly created, which decides between HTTP 201 and 200:
    ///
    /// > *HTTP `200 - Ok` when the object already existed and has successfully been updated.
    /// > HTTP `201 - Created` when the object has been newly created in the server system.*
    fn put_location(
        &self,
        owner: PartyRef,
        location: crate::v2_3_0::locations::Location,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Created>> + Send;

    /// `PUT {locations}/{cc}/{party}/{location_id}/{evse_uid}`.
    fn put_evse(
        &self,
        owner: PartyRef,
        location_id: String,
        evse: crate::v2_3_0::locations::Evse,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Created>> + Send;

    /// `PUT {locations}/{cc}/{party}/{location_id}/{evse_uid}/{connector_id}`.
    fn put_connector(
        &self,
        owner: PartyRef,
        location_id: String,
        evse_uid: String,
        connector: crate::v2_3_0::locations::Connector,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Created>> + Send;

    /// `PATCH` on any of the three levels.
    ///
    /// The patch is guaranteed to carry `last_updated`; the extractor refuses one that does not.
    fn patch(
        &self,
        owner: PartyRef,
        location_id: String,
        evse_uid: Option<String>,
        connector_id: Option<String>,
        patch: Patch<serde_json::Value>,
        context: RequestContext,
    ) -> impl Future<Output = Handled<()>> + Send;
}

/// Whether a `PUT` created the object or replaced one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Created {
    /// The object did not exist; answer with HTTP 201.
    Yes,
    /// The object existed and was replaced; answer with HTTP 200.
    No,
}

impl Created {
    /// The HTTP status this outcome maps to.
    #[must_use]
    pub const fn http_status(self) -> u16 {
        match self {
            Self::Yes => 201,
            Self::No => 200,
        }
    }
}

impl From<bool> for Created {
    /// `true` means the object was newly created.
    fn from(created: bool) -> Self {
        if created { Self::Yes } else { Self::No }
    }
}

// ---------------------------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------------------------

/// The eMSP side of the Tokens module: serving Tokens and answering real-time authorizations.
///
/// Spec: 2.3.0 §mod_tokens_emsp_interface
pub trait TokensSender: Send + Sync + 'static {
    /// `GET {tokens}` — one page of Tokens.
    fn list(
        &self,
        query: PageQuery,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Page<crate::v2_3_0::tokens::Token>>> + Send;

    /// `POST {tokens}/{token_uid}/authorize[?type=]` — a real-time authorization.
    ///
    /// > *`2004 Unknown Token`* is the code for a token this party does not know; return
    /// > [`OcpiError::Remote`] with that code, or an
    /// > [`AuthorizationInfo`](crate::v2_3_0::tokens::AuthorizationInfo) with
    /// > [`AllowedType::NotAllowed`](crate::v2_3_0::tokens::AllowedType::NotAllowed) when the
    /// > token is known but may not charge here.
    fn authorize(
        &self,
        token_uid: String,
        token_type: Option<crate::v2_3_0::tokens::TokenType>,
        location: Option<crate::v2_3_0::tokens::LocationReferences>,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::tokens::AuthorizationInfo>> + Send;
}

/// The CPO side of the Tokens module: receiving the Tokens an eMSP owns.
///
/// Spec: 2.3.0 §mod_tokens_cpo_interface
pub trait TokensReceiver: Send + Sync + 'static {
    /// `GET {tokens}/{cc}/{party}/{token_uid}[?type=]`.
    fn token(
        &self,
        owner: PartyRef,
        token_uid: String,
        token_type: Option<crate::v2_3_0::tokens::TokenType>,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::tokens::Token>> + Send;

    /// `PUT {tokens}/{cc}/{party}/{token_uid}[?type=]`.
    fn put_token(
        &self,
        owner: PartyRef,
        token: crate::v2_3_0::tokens::Token,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Created>> + Send;

    /// `PATCH {tokens}/{cc}/{party}/{token_uid}[?type=]`.
    fn patch_token(
        &self,
        owner: PartyRef,
        token_uid: String,
        token_type: Option<crate::v2_3_0::tokens::TokenType>,
        patch: Patch<crate::v2_3_0::tokens::Token>,
        context: RequestContext,
    ) -> impl Future<Output = Handled<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// CDRs and Sessions
// ---------------------------------------------------------------------------------------------

/// The CPO side of the CDRs module.
///
/// Spec: 2.3.0 §mod_cdrs_cpo_interface
pub trait CdrsSender: Send + Sync + 'static {
    /// `GET {cdrs}` — one page of CDRs.
    fn list(
        &self,
        query: PageQuery,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Page<crate::v2_3_0::cdrs::Cdr>>> + Send;
}

/// The eMSP side of the CDRs module.
///
/// The only OCPI module where `POST` creates a server-owned object:
///
/// > *The eMSP returns the URL to the just created CDR object in the `Location` header field.*
///
/// Spec: 2.3.0 §mod_cdrs_emsp_interface
pub trait CdrsReceiver: Send + Sync + 'static {
    /// `GET {cdrs}/{cdr_id}` — a CDR previously POSTed here.
    fn cdr(
        &self,
        cdr_id: String,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::cdrs::Cdr>> + Send;

    /// `POST {cdrs}` — stores a CDR and returns the URL it can be fetched from.
    fn post_cdr(
        &self,
        cdr: crate::v2_3_0::cdrs::Cdr,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::types::Url>> + Send;
}

/// The CPO side of the Sessions module.
///
/// Spec: 2.3.0 §mod_sessions_cpo_interface
pub trait SessionsSender: Send + Sync + 'static {
    /// `GET {sessions}` — one page of Sessions.
    fn list(
        &self,
        query: PageQuery,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Page<crate::v2_3_0::sessions::Session>>> + Send;

    /// `PUT {sessions}/{session_id}/charging_preferences`.
    ///
    /// > *If a PUT with ChargingPreferences is received for an EVSE that does not have the
    /// > capability `CHARGING_PREFERENCES_CAPABLE`, the receiver should respond with an HTTP
    /// > status of 404 and an OCPI status code of 2001.*
    fn set_charging_preferences(
        &self,
        session_id: String,
        preferences: crate::v2_3_0::sessions::ChargingPreferences,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::sessions::ChargingPreferencesResponse>> + Send;
}

/// The eMSP side of the Sessions module.
///
/// Spec: 2.3.0 §mod_sessions_emsp_interface
pub trait SessionsReceiver: Send + Sync + 'static {
    /// `GET {sessions}/{cc}/{party}/{session_id}`.
    fn session(
        &self,
        owner: PartyRef,
        session_id: String,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::sessions::Session>> + Send;

    /// `PUT {sessions}/{cc}/{party}/{session_id}`.
    fn put_session(
        &self,
        owner: PartyRef,
        session: crate::v2_3_0::sessions::Session,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Created>> + Send;

    /// `PATCH {sessions}/{cc}/{party}/{session_id}`.
    fn patch_session(
        &self,
        owner: PartyRef,
        session_id: String,
        patch: Patch<crate::v2_3_0::sessions::Session>,
        context: RequestContext,
    ) -> impl Future<Output = Handled<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Tariffs
// ---------------------------------------------------------------------------------------------

/// The CPO side of the Tariffs module.
///
/// Spec: 2.3.0 §mod_tariffs_cpo_interface
pub trait TariffsSender: Send + Sync + 'static {
    /// `GET {tariffs}` — one page of Tariffs.
    fn list(
        &self,
        query: PageQuery,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Page<crate::v2_3_0::tariffs::Tariff>>> + Send;
}

/// The eMSP side of the Tariffs module.
///
/// The only client-owned module with a `DELETE`: a tariff that no longer exists is simply gone,
/// unlike an EVSE, which is retired with a status.
///
/// Spec: 2.3.0 §mod_tariffs_emsp_interface
pub trait TariffsReceiver: Send + Sync + 'static {
    /// `GET {tariffs}/{cc}/{party}/{tariff_id}`.
    fn tariff(
        &self,
        owner: PartyRef,
        tariff_id: String,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::tariffs::Tariff>> + Send;

    /// `PUT {tariffs}/{cc}/{party}/{tariff_id}`.
    fn put_tariff(
        &self,
        owner: PartyRef,
        tariff: crate::v2_3_0::tariffs::Tariff,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Created>> + Send;

    /// `DELETE {tariffs}/{cc}/{party}/{tariff_id}`.
    fn delete_tariff(
        &self,
        owner: PartyRef,
        tariff_id: String,
        context: RequestContext,
    ) -> impl Future<Output = Handled<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

/// The CPO side of the Commands module: receiving commands from an eMSP.
///
/// A command is answered twice: immediately with a
/// [`CommandResponse`](crate::v2_3_0::commands::CommandResponse) carrying a timeout, and later by
/// POSTing a [`CommandResult`](crate::v2_3_0::commands::CommandResult) to the command's
/// `response_url`. This trait covers the first; the second is an outgoing request the
/// implementation makes when the Charge Point answers.
///
/// **Check the `response_url` before calling it.** It comes from the peer, and a party that
/// fetches it unconditionally is an SSRF proxy; the [`client`](crate::client) does this check for
/// you.
///
/// Spec: 2.3.0 §mod_commands_commands_module
pub trait CommandsReceiver: Send + Sync + 'static {
    /// `POST {commands}/{command}`.
    fn command(
        &self,
        command: crate::v2_3_0::commands::Command,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::commands::CommandResponse>> + Send;
}

/// The eMSP side of the Commands module: receiving the asynchronous result.
///
/// Spec: 2.3.0 §mod_commands_commands_module
pub trait CommandsSender: Send + Sync + 'static {
    /// `POST {response_url}` — the Charge Point's eventual answer.
    ///
    /// The `unique_id` is whatever the implementation put in the `response_url` when it sent the
    /// command, which is how the result is matched to the request:
    ///
    /// > *This URL might contain a unique ID to be able to distinguish between StartSession
    /// > requests.*
    fn command_result(
        &self,
        unique_id: String,
        result: crate::v2_3_0::commands::CommandResult,
        context: RequestContext,
    ) -> impl Future<Output = Handled<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------------------------

/// The credentials module, which every implementation must serve.
///
/// The four methods are the registration lifecycle. The two 405 rules are the ones most often
/// missed, and the router does not enforce them for you because only the implementation knows
/// whether a peer is registered:
///
/// > *POST … MUST return a HTTP status code 405: method not allowed if the client has already
/// > been registered before.*
/// >
/// > *PUT … MUST return a HTTP status code 405: method not allowed if the client has not been
/// > registered yet.*
///
/// Return [`OcpiError::MethodNotAllowed`] for those;
/// [`PeerState`](crate::client::PeerState) has the predicates.
///
/// Spec: 2.3.0 §credentials_credentials_endpoint
pub trait CredentialsHandler: Send + Sync + 'static {
    /// `GET {credentials}` — this party's own credentials object.
    fn get(
        &self,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::credentials::Credentials>> + Send;

    /// `POST {credentials}` — register.
    ///
    /// The implementation must fetch the client's versions and version details with the token in
    /// `credentials` **before** answering, and answer `3001` if that fails:
    ///
    /// > *When the initializing party requests data from the other party during the open POST call
    /// > to its credentials endpoint. If one of the GETs can not be processed, the party should
    /// > return this error in the POST response.*
    fn post(
        &self,
        credentials: crate::v2_3_0::credentials::Credentials,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::credentials::Credentials>> + Send;

    /// `PUT {credentials}` — update, rotate the token, or switch version.
    ///
    /// > *The server must fetch the client's endpoints again, even if the version has not
    /// > changed.*
    fn put(
        &self,
        credentials: crate::v2_3_0::credentials::Credentials,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::credentials::Credentials>> + Send;

    /// `DELETE {credentials}` — unregister.
    ///
    /// > *Both parties must end any automated communication.*
    fn delete(&self, context: RequestContext) -> impl Future<Output = Handled<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Hub Client Info
// ---------------------------------------------------------------------------------------------

/// The hub side of the Hub Client Info module.
///
/// A configuration module: its requests carry no routing headers.
///
/// Spec: 2.3.0 §mod_hub_client_info_module
pub trait HubClientInfoSender: Send + Sync + 'static {
    /// `GET {hubclientinfo}` — one page of `ClientInfo` objects.
    fn list(
        &self,
        query: PageQuery,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Page<crate::v2_3_0::hub_client_info::ClientInfo>>> + Send;
}

/// The client side of the Hub Client Info module.
///
/// Spec: 2.3.0 §mod_hub_client_info_module
pub trait HubClientInfoReceiver: Send + Sync + 'static {
    /// `GET {hubclientinfo}/{cc}/{party}`.
    fn client_info(
        &self,
        party: PartyRef,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::hub_client_info::ClientInfo>> + Send;

    /// `PUT {hubclientinfo}/{cc}/{party}`.
    fn put_client_info(
        &self,
        party: PartyRef,
        info: crate::v2_3_0::hub_client_info::ClientInfo,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Created>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Charging Profiles
// ---------------------------------------------------------------------------------------------

/// The CPO side of the Charging Profiles module: accepting profiles for a running session.
///
/// Every method here answers **twice**. The `ChargingProfileResponse` returned from the method is
/// the CPO's own immediate answer — did it understand the request and manage to pass it to the
/// EVSE — and, when that answer is `ACCEPTED`, the Charge Point's eventual verdict follows as a
/// POST to the `response_url` the Sender supplied.
///
/// > *The response contains the direct response from the Receiver (Typically CPO), not the
/// > response from the EVSE itself, that will be sent via an asynchronous POST on the Sender
/// > interface if this response is `ACCEPTED`.*
///
/// **Check the `response_url` before calling it.** It comes from the peer; see
/// [`UrlPolicy`](crate::types::UrlPolicy).
///
/// Spec: 2.3.0 §mod_charging_profiles_cpo_interface
pub trait ChargingProfilesReceiver: Send + Sync + 'static {
    /// `GET {chargingprofiles}/{session_id}?duration={duration}&response_url={url}`.
    ///
    /// The active profile itself is not returned here — it arrives at `response_url` as an
    /// [`ActiveChargingProfileResult`](crate::v2_3_0::charging_profiles::ActiveChargingProfileResult).
    fn active_charging_profile(
        &self,
        session_id: String,
        duration_seconds: u64,
        response_url: crate::types::Url,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::charging_profiles::ChargingProfileResponse>> + Send;

    /// `PUT {chargingprofiles}/{session_id}` with a `SetChargingProfile` body.
    fn set_charging_profile(
        &self,
        session_id: String,
        request: crate::v2_3_0::charging_profiles::SetChargingProfile,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::charging_profiles::ChargingProfileResponse>> + Send;

    /// `DELETE {chargingprofiles}/{session_id}?response_url={url}`.
    fn clear_charging_profile(
        &self,
        session_id: String,
        response_url: crate::types::Url,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::charging_profiles::ChargingProfileResponse>> + Send;
}

/// The eMSP/SCSP side of the Charging Profiles module: receiving what the Charge Point decided.
///
/// # Why the callback paths are three, not one
///
/// The specification leaves the `response_url` entirely to the Sender —
///
/// > *No structure defined. This is open to the eMSP to define, the URL is provided to the
/// > Receiver by the Sender.*
///
/// — and that freedom is load-bearing here, because the three result bodies are **not**
/// distinguishable from one another:
/// [`ChargingProfileResult`](crate::v2_3_0::charging_profiles::ChargingProfileResult) and
/// [`ClearProfileResult`](crate::v2_3_0::charging_profiles::ClearProfileResult) have identical
/// shapes, one `result` field each. A single endpoint that sniffed the body could not tell a
/// rejected PUT from a rejected DELETE.
///
/// So [`OcpiRouter::charging_profiles_sender`](crate::server::OcpiRouter::charging_profiles_sender)
/// mounts one path per result kind, each ending in the Sender's own unique id, and
/// [`CallbackUrls`](crate::server::CallbackUrls) builds the matching `response_url`s. The kind is
/// then carried by the URL, exactly as the specification intends.
///
/// Spec: 2.3.0 §mod_charging_profiles_emsp_interface
pub trait ChargingProfilesSender: Send + Sync + 'static {
    /// The Charge Point's answer to a GET of the active profile.
    fn active_charging_profile_result(
        &self,
        unique_id: String,
        result: crate::v2_3_0::charging_profiles::ActiveChargingProfileResult,
        context: RequestContext,
    ) -> impl Future<Output = Handled<()>> + Send;

    /// The Charge Point's answer to a PUT of a charging profile.
    fn charging_profile_result(
        &self,
        unique_id: String,
        result: crate::v2_3_0::charging_profiles::ChargingProfileResult,
        context: RequestContext,
    ) -> impl Future<Output = Handled<()>> + Send;

    /// The Charge Point's answer to a DELETE of a charging profile.
    fn clear_profile_result(
        &self,
        unique_id: String,
        result: crate::v2_3_0::charging_profiles::ClearProfileResult,
        context: RequestContext,
    ) -> impl Future<Output = Handled<()>> + Send;

    /// `PUT {chargingprofiles}/{session_id}` — the CPO volunteering a changed active profile.
    ///
    /// > *The Receiver SHALL call this interface every time it knows changes have been made that
    /// > influence the ActiveChargingProfile for an ongoing session AND the Sender has at least
    /// > once successfully called the charging profile Receiver PUT interface for this session.*
    fn put_active_charging_profile(
        &self,
        session_id: String,
        profile: crate::v2_3_0::charging_profiles::ActiveChargingProfile,
        context: RequestContext,
    ) -> impl Future<Output = Handled<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Payments
// ---------------------------------------------------------------------------------------------

/// The PTP side of the Payments module: the terminals this Payment Terminal Provider owns.
///
/// Note the direction. In Payments the **PTP** is the Sender — it owns the `Terminal` objects —
/// and the CPO drives them, which is why activation and location assignment are writes *on this
/// interface* rather than pushes to the CPO.
///
/// Spec: 2.3.0 §mod_payments_ptp_interface
pub trait PaymentsSender: Send + Sync + 'static {
    /// `GET {payments}/terminals` — one page of Terminals.
    fn terminals(
        &self,
        query: PageQuery,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Page<crate::v2_3_0::payments::Terminal>>> + Send;

    /// `GET {payments}/terminals/{terminal_id}`.
    fn terminal(
        &self,
        terminal_id: String,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::payments::Terminal>> + Send;

    /// `PUT {payments}/terminals/{terminal_id}` — the CPO updating a terminal's location data.
    fn put_terminal(
        &self,
        terminal_id: String,
        terminal: crate::v2_3_0::payments::Terminal,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::payments::Terminal>> + Send;

    /// `PATCH {payments}/terminals/{terminal_id}`.
    ///
    /// > *This PATCH should be used by the CPO to assign location ids and/or evse_uids to a
    /// > terminal.*
    fn patch_terminal(
        &self,
        terminal_id: String,
        patch: Patch<crate::v2_3_0::payments::Terminal>,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::payments::Terminal>> + Send;

    /// `POST {payments}/terminals/activate`.
    ///
    /// > *NOTE: The terminal_id is optional in the activation request as it will be set by the
    /// > PTP. The cardinality for the remaining fields stays the same.*
    ///
    /// A `Terminal` without its `terminal_id` is not a `Terminal`, so the body arrives as a
    /// [`Patch`] — this crate's type for "an OCPI object with fields left out". Note that it is
    /// **not** a merge patch: this is a `POST`, nothing is being merged into anything, and the
    /// rule that a `PATCH` must carry `last_updated` deliberately does not apply. Read the
    /// fields with [`Patch::as_value`]; do not call [`Patch::apply`].
    fn activate_terminal(
        &self,
        terminal: Patch<crate::v2_3_0::payments::Terminal>,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::payments::Terminal>> + Send;

    /// `POST {payments}/terminals/{terminal_id}/deactivate`.
    fn deactivate_terminal(
        &self,
        terminal_id: String,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::payments::Terminal>> + Send;

    /// `GET {payments}/financial-advice-confirmations` — one page.
    fn financial_advice_confirmations(
        &self,
        query: PageQuery,
        context: RequestContext,
    ) -> impl Future<Output = Handled<Page<crate::v2_3_0::payments::FinancialAdviceConfirmation>>> + Send;

    /// `GET {payments}/financial-advice-confirmations/{id}`.
    fn financial_advice_confirmation(
        &self,
        id: String,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::payments::FinancialAdviceConfirmation>> + Send;
}

/// The CPO side of the Payments module: the PTP's terminals as the CPO stores them.
///
/// Unusually for OCPI, these are **POSTs, not PUTs**, and the URL carries no owning party:
///
/// > *The POST should be used by the PTP to create a newly shipped terminal on the CPO's system.*
///
/// Spec: 2.3.0 §mod_payments_cpo_interface
pub trait PaymentsReceiver: Send + Sync + 'static {
    /// `GET {payments}/terminals/{terminal_id}` — what the CPO has stored.
    fn terminal(
        &self,
        terminal_id: String,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::payments::Terminal>> + Send;

    /// `POST {payments}/terminals` — the PTP creating a terminal in the CPO's system.
    fn post_terminal(
        &self,
        terminal: crate::v2_3_0::payments::Terminal,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::payments::Terminal>> + Send;

    /// `GET {payments}/financial-advice-confirmations/{id}`.
    fn financial_advice_confirmation(
        &self,
        id: String,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::payments::FinancialAdviceConfirmation>> + Send;

    /// `POST {payments}/financial-advice-confirmations`.
    ///
    /// > *The PTP has to make sure to use the same authorization reference as provided in the
    /// > Commands.StartSession so that the CPO can properly map the financial advice to the
    /// > session.*
    fn post_financial_advice_confirmation(
        &self,
        confirmation: crate::v2_3_0::payments::FinancialAdviceConfirmation,
        context: RequestContext,
    ) -> impl Future<Output = Handled<crate::v2_3_0::payments::FinancialAdviceConfirmation>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn created_maps_to_the_status_the_spec_gives() {
        assert_eq!(Created::Yes.http_status(), 201);
        assert_eq!(Created::No.http_status(), 200);
        assert_eq!(Created::from(true), Created::Yes);
        assert_eq!(Created::from(false), Created::No);
    }
}
