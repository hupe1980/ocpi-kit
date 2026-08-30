+++
title = "Parse, validate, construct"
weight = 10
description = "Why decoding, conformance checking and object construction are three separate questions in ocpi-kit."
+++

One rule governs the whole crate:

> **Parse permissively. Validate explicitly. Construct strictly.**

Each of the three is a different job, and conflating them is how OCPI libraries become
unusable in production.

## Parse permissively

A peer will send you a `string(36)` with 40 characters in it. It will send a `ConnectorType` that
was added to the spec after your last release. It will send a field you have never heard of.

If decoding rejects those, one bad object makes a whole page of 200 Locations undecodable, and you
cannot even see what was wrong. So decoding in `ocpi-kit` accepts what it is given:

* over-long strings arrive intact
* unknown enum values keep their text (see [Enums](@/docs/concepts/enums.md))
* unknown fields are kept (see [Extensions](@/docs/concepts/extensions.md))

Decoding fails only when the JSON genuinely does not fit the shape — a required field missing, an
object where an array belongs.

## Validate explicitly

Conformance is a separate question you ask when you want the answer:

```rust
use ocpi_kit::types::Validate;

if let Err(violations) = location.validate() {
    for v in violations.iter() {
        eprintln!("{} {:?}: {}", v.pointer, v.code, v.message);
    }
}
```

Every violation carries an **RFC 6901 JSON Pointer** into the document — `/evses/2/connectors/0/
standard`, not "somewhere in the connectors". The pointer is built as validation descends, with
`~0`/`~1` escaping, so it can be pasted into any JSON tool.

`ViolationCode` says what kind of problem it is:

| Code | Meaning |
|---|---|
| `TooLong` | a `string(N)` or `CiString(N)` over its limit |
| `IllegalCharacter` | a `CiString` containing something outside its character set |
| `EmptyRequiredList` | a list the spec gives cardinality `+` |
| `OutOfRange` | a numeric or temporal value outside its documented range |
| `Inconsistent` | two fields that contradict each other |
| `MissingConditional` | a field required because of another field's value |
| `Imprecise` | a decimal that would not survive a JSON round-trip |

Validation is recursive and reports *everything*, not just the first problem, so one pass produces
a complete list.

### Where validation happens for you

* The **client** validates outgoing objects by default (`ClientConfig::validate_outgoing`), so a
  non-conformant object is caught at your process boundary rather than at a partner's support desk.
* The **server** validates incoming bodies and answers with the specification's own status codes.

Both are settings, because a hub that must relay a slightly-wrong object unchanged is a legitimate
use.

## Construct strictly

When *you* build an object, the crate stops you from making a bad one. Constructors return
`Result`:

```rust
let party = PartyRef::new("NL", "TNM")?;   // checks the country code and party id
let url = Url::new("https://…")?;          // checks the scheme and the policy
let token = CredentialsToken::new("…")?;   // checks it is usable
```

`From<&str>` is deliberately the *lenient* path — it exists so builders can take `"NL"` without
ceremony, and so a value that came off the wire can be re-wrapped without a spurious failure. The
strictness guarantee lives in `new()`, `FromStr` and outgoing validation.
