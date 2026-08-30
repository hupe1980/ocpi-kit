+++
title = "Spec errata"
weight = 30
description = "Places where the OCPI specification contradicts itself, and how ocpi-kit handles each one."
+++

Places where the OCPI specification contradicts itself, and what `ocpi-kit` does about each.

Every one is a test in
[`tests/fixtures.rs`](https://github.com/hupe1980/ocpi-kit/blob/main/tests/fixtures.rs) marked
`Expect::Erratum` with the reason, so if upstream fixes one the test fails and says to promote it.

## In the specification's own examples

| Example | Problem |
|---|---|
| `cdr_example.json` (2.3.0) | The Tariff embedded in `tariffs[0]` omits `tax_included`, which 2.3.0 adds as a required field (cardinality 1) of the Tariff object |
| `location_put_example_add_evse.json` (2.2.1 **and** 2.3.0) | Uses `floor` where the EVSE field is `floor_level`, and gives `floor` and `physical_reference` as JSON numbers where the spec says `string(16)` |
| `tariff_put_example.json` (2.2.1 and 2.3.0) | A PUT body "must specify all required fields of an object" (§transport_and_format_put), but this one omits `last_updated` |
| `payment_financial_advice_confirmation_*.json` (3 files) | `total_costs` is written in the OCPI 2.2.1 `Price` shape `{excl_vat, incl_vat}`; 2.3.0 replaced it with `{before_taxes, taxes[]}` |
| `cdrs_example_of_a_cdr.json` (2.1.1) | The embedded Tariff writes `price` as the JSON *string* `"2.00"` where §types_number_type requires a number. Recorded as `Tolerated`: this crate parses it exactly and emits it unquoted |
| `location_example.json`, `location_example_parking_garage_opening_hours.json` (bookings branch) | Give `EVSE.parking` as a list of bare id strings, although the branch's *own* property table defines it as `EVSEParking*`, identically to core 2.3.0 |
| `booking_example.json` (bookings branch) | Every `booking_requests[].booking_request` omits `booking_location_id`, which the BookingRequest table gives cardinality 1; and `party_id` is `INF12` where the table says `CiString(3)` |
| `booking_location_example.json` (bookings branch) | `booking_option.evse_position` is a single string, but the BookingOption table gives it cardinality `*` — a list |

## In the text

| Where | Problem | How `ocpi-kit` handles it |
|---|---|---|
| `mod_hub_client_info` (2.2.1 and 2.3.0) | The Sender GET endpoint structure is written as `{locations_endpoint_url}?…` — a copy-paste from Locations | Treated as `{hubclientinfo_endpoint_url}` |
| `mod_hub_client_info` receiver PUT | The example URL uses version `2.0` and path `clientinfo`, though the module id is `hubclientinfo` | URL builders use the discovered endpoint URL; examples are not normative |
| `mod_bookings` | The module identifier is literally `Booking`, where every other id is a lowercase plural; the receiver-interface tables link to `mod_locations` anchors | `ModuleId::Booking` matches `bookings` case-insensitively, as a documented interop accommodation |
| The `payments` branch `ModuleID` table | Does not list `payments` at all, although the module exists and its endpoints are specified | `ModuleId::Payments` exists and documents the omission |
| 2.3.0 `credentials` examples | No example shows `hub_party_id`, although the text mandates it for routing platforms | Our fixtures add one, marked non-spec |
| `types.asciidoc` `string(N)` | Silent on whether `N` counts bytes or characters | This crate counts Unicode scalar values, and is lenient on ingest |
| `mod_tariffs` | Explicitly states there are no rounding rules, and notes `step_size` is removed in OCPI 3.0 | Both are settings on `PricingPolicy` |
| Payments | `Terminal` PUT/PATCH/GET-one URL structures are only given as "a `terminal_id` URL segment", never as a full pattern, and the examples still show `/2.2.1/` paths | Builders append to the discovered endpoint, the only reading consistent with §transport_and_format_interface_endpoints |
