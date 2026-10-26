# Template Approval and Deployment Integrity Plan

Status: The controlled approval path is implemented through the Slice 6
client guards and the Slice 7 trust-root rotation contract. The default
runtime remains `inspection_only`; enforcement is enabled explicitly with a
validated approval bundle until real-host integration evidence is recorded.
The repository now contains
inspection/identity contracts, signed receipts, immutable deployment snapshots,
and an explicit `runtime serve-approved` path that exposes only the gateway
after all admission checks. The ordinary TUI launch path remains
`inspection_only` and does not silently switch to this path.

Until the launch gate and request gate are wired into the ordinary TUI and
migrated clients, LMML must describe the default feature as **template
inspection**, not globally enforced template approval.

The implemented contract boundary is in
`crates/lmml-substrate/src/approval.rs`. It includes deterministic,
domain-separated prompt/runtime/deployment hashes, source template inspection,
Ed25519-signed receipts with no unsigned fallback, and a separately anchored
revocation state. It also includes content-addressed deployment snapshot
publication and a computed admission state that invalidates identity changes.
It includes a versioned request validator that rejects request-level template
and parser overrides. The new `crates/lmml-gateway` crate provides the
fail-closed transport boundary and streaming revocation behavior; it forwards
to the backend only with a private bearer key. `lmml-server::ServerHandle` can
own and shut down that gateway alongside the backend child. The explicit
`lmml runtime serve-approved` command activates this path only after receipt,
revocation, snapshot, model, argv, executable, and identity checks. The TUI
can supervise the bundle path when `LMML_APPROVAL_ENFORCEMENT=1` and
`LMML_APPROVAL_BUNDLE` are set; external clients still require explicit
gateway configuration until Slice 6 is complete.

Source inspection is available without enabling approval:

```text
lmml model inspect-template /path/to/model --json
# Add --deployed model.gguf to compare embedded GGUF template variants.
```

The command reports source container, named-template, external-template, and
special-token hashes and explicitly reports `inspection_only`.

The isolated enforcement path is invoked explicitly with
`lmml runtime serve-approved`. It requires the signed prompt, runtime, and
deployment records, the pinned trust root, the content-addressed snapshot
root, the exact model and binary, and a private backend bearer key. It may
also receive a signed revocation state. It verifies those identities before
starting llama-server on a private loopback port and exposes only the gateway
port. `lmml model approve-template --bundle-out` can persist these references
as one immutable managed bundle, and `lmml runtime serve-approved-bundle`
resolves that bundle without accepting loose model or binary paths. These
commands are integration targets for Slice 5; they are not yet the default
TUI/client launch path.

Both approved runtime commands accept `--json`; ready evidence includes the
endpoint, process ID, receipt ID and sequence, prompt/runtime/deployment
hashes, model hash, and executable hash.

For controlled TUI migration testing, set the bundle and backend secret before
starting `lmml`:

```sh
export LMML_APPROVAL_BUNDLE=/path/to/approval-bundle.json
export LMML_BACKEND_API_KEY='private-backend-token'
export LMML_APPROVAL_ENFORCEMENT=1
export LMML_GATEWAY_ID=gateway-terran
export LMML_GATEWAY_DEPLOYMENT_HASH=sha256:...
export LMML_GATEWAY_RECEIPT_ID=receipt-...
export LMML_GATEWAY_RECEIPT_SEQUENCE=1
export LMML_GATEWAY_REVOCATION_EPOCH=0
# Hex-encoded PKCS#8 Ed25519 key held only by the gateway process.
export LMML_GATEWAY_ATTESTATION_PRIVATE_KEY_HEX=...
lmml
```

The corresponding raw public key must be provisioned into each enforced node
and router as `LMML_GATEWAY_ATTESTATION_PUBLIC_KEY_HEX`. The private key is an
operator trust-root input and must not be placed in a bundle, model directory,
or client configuration. For a new key pair, provision the PKCS#8 bytes and
raw 32-byte Ed25519 public key with a protected operator workflow, for example:

```sh
openssl genpkey -algorithm Ed25519 -out gateway-attestation.pem
export LMML_GATEWAY_ATTESTATION_PRIVATE_KEY_HEX="$(openssl pkey -in gateway-attestation.pem -outform DER | xxd -p -c 99999)"
export LMML_GATEWAY_ATTESTATION_PUBLIC_KEY_HEX="$(openssl pkey -in gateway-attestation.pem -pubout -outform DER | tail -c 32 | xxd -p -c 99999)"
```

Key rotation requires restarting the gateway, nodes, and router with the new
public key after the deployment is re-approved; there is no unsigned or
implicit key fallback.

With enforcement enabled, the TUI refuses every direct backend launch and
supervises `serve-approved-bundle` instead. Unset the switch to retain legacy
inspection-only behavior while client migration is incomplete.

`lmml-node` uses the same switch to require a valid, short-lived,
Ed25519-signed gateway attestation on the configured upstream health endpoint;
the public key is supplied through
`LMML_GATEWAY_ATTESTATION_PUBLIC_KEY_HEX`. Nodes and routers must also receive
the five `LMML_GATEWAY_*` binding values above; they reject signatures whose
gateway, deployment, receipt, sequence, or revocation epoch differs. A raw
llama-server upstream or a
forged `x-lmml-approved-upstream` marker is rejected before the node starts.
The node repeats this check before every forwarded inference request, and
`lmml-router` verifies the same attestation on every discovery refresh.

The gateway backend URL is restricted to an unauthenticated loopback HTTP
endpoint. The public listener is not the backend listener. The gateway starts
in `blocked` state, so its health and inference routes remain unavailable
until model bytes and loaded-library checks complete; only then is admission
atomically activated. In enforced mode, a missing attestation key is a launch
failure rather than an unsigned fallback.

Signed receipts can be produced for review records with
`lmml model approve-template`. Receipt creation does not enable the ordinary
TUI launch path; the default client integration remains disabled until the
private backend and all client migration gates pass.

`lmml model template-status` recomputes the bound identities and verifies the
receipt/trust-root/revocation state without launching a process. Supplying
`--snapshot-root` also re-hashes every deployed file before reporting status.
Its non-zero exit status is suitable for client and installer preflight checks.

Trust-root rotation is explicit and old-key authorized:

```sh
lmml model rotate-approval-trust-root \
  --old-root current-root.json \
  --old-private-key current-key.pkcs8 \
  --new-key-id operator-epoch-2 \
  --new-root-out next-root.json \
  --new-private-key-out next-key.pkcs8 \
  --migration-out trust-root-rotation.json \
  --sequence 2
```

The replacement root is accepted only when the old key signs the migration and
the key epoch advances. Existing receipts are not silently reinterpreted under
the new root.

For migration testing, `LMML_APPROVAL_ENFORCEMENT=1` makes ordinary TUI server
actions and detached runtime-profile starts fail closed with an instruction to
use the approved gateway path. This is a guardrail, not a substitute for
Slice 6 client migration.

## Release Invariant

The feature is enabled only when both conditions are true:

1. An unapproved effective prompt configuration cannot become reachable through
2. A change to an approved prompt or runtime configuration invalidates
   admission before another request is accepted.

The invariant applies to model bytes, tokenizer assets, template bytes,
multimodal assets, the renderer, the backend executable and libraries, the
launch environment, and request-level template controls.

Until the private backend transport and request gateway are operational, all
template work remains `inspection_only`. A signed approval must not make a raw
`llama-server` endpoint externally reachable. Enforced approval begins only
after Slice 6 client migration and integration tests pass.

## 1. Threat Model

### Trusted local operator

The initial deployment assumes the local operator owns the LMML data directory,
can start and stop LMML, and is authorized to review and approve a model
configuration. The operator may intentionally approve unsafe content; approval
does not make that content safe.

The trusted-operator assumption permits protected managed storage and a signing
key only after the trust root is selected. The recommended default is an
operator-provisioned key, with the public key pinned in protected LMML state
and the private key held outside the managed data directory. A locally
generated key is acceptable only when the policy explicitly accepts
equivalent-user compromise.

Key management must define rotation, backup, recovery, and public-key pinning.
Replacing the key is a policy migration requiring explicit operator action; it
cannot silently authorize old receipts. OS keyring or TPM-backed storage is
preferred where available, with the operator-provisioned key as the portable
fallback.

### Untrusted clients and processes

Clients, coding harnesses, OpenCode, `lmml-node` callers, router callers, and
model-generated tool calls are untrusted with respect to template policy. They
may submit messages, tools, permitted request parameters, and inference
requests, but may not choose a template, parser, renderer, model path, runtime
binary, library, or deployment directory.

Local processes that can reach a backend socket are also untrusted. A process
must not bypass LMML by calling a managed `llama-server` directly.

### Explicit exclusions

Template approval does not certify:

- model weights, alignment, or provenance beyond the bound artifact identity;
- prompt-injection resistance;
- generated-output correctness or harmlessness;
- tool-execution safety;
- arbitrary Jinja resource safety outside the restricted renderer boundary;
- a model's behavior on inputs not covered by the approval test suite.

Approval establishes only that LMML reviewed and admitted one exact
prompt-rendering and runtime configuration.

### Enforced guarantees

When enabled, LMML guarantees that:

- the effective configuration was resolved before approval lookup;
- the approved bytes are the bytes deployed;
- the deployed process and loaded libraries match the admitted identities;
- unsupported request controls are rejected;
- revocation blocks new requests;
- approval-store failure fails closed;
- no direct backend endpoint is exposed as an alternate policy path.

## 2. Verified Current Architecture

The following is based on the current repository, not the earlier review text.

### Import and derivation

- `crates/lmml-tui/src/model_cli.rs` defines `ModelCommand::Import`,
  `Verify`, `Inspect`, `Derive`, `AdmitArtifact`, and candidate commands.
- Safetensors import and canonical manifest creation are implemented in the
  `lmml-substrate` import/storage modules.
- GGUF derivation is orchestrated by
  `crates/lmml-tui/src/model_cli/derive.rs`.
- Conversion and quantization use streamed Tokio subprocesses, temporary
  destination files, GGUF admission, SHA-256 hashing, and append-only artifact
  records.
- `crates/lmml-tui/src/model_cli/derive/admission.rs` validates GGUF structure
  and can use `llama-server` to open the artifact before registration.
- The current substrate types distinguish `ModelIdentity`, `ArtifactIdentity`,
  `ArtifactManifest`, `GgufCandidateManifest`, and `ArtifactAdmission`.
- `crates/lmml-tui/src/model_cli.rs` exposes `model inspect-template` for
  source and deployed GGUF metadata inspection and `model approve-template`
  for signed review records. These commands remain inspection-only; they do
  not make a raw backend reachable.

### Configuration resolution

- `crates/lmml-tui/src/app.rs::server_config` assembles the final compat
  configuration for a selected model.
- `server_settings_for_model` applies a model runtime profile.
- `resolved_profile_server` resolves known sidecar paths, including `--mmproj`,
  but does not create a deployment snapshot or approval identity.
- `apply_qwen_server_safeguards` and `apply_prism_initial_safeguards` mutate
  the final configuration after profile selection. These generated changes
  must be included in any future effective-configuration hash.
- `crates/lmml-compat/src/lib.rs::build_argv` converts
  `lmml_compat::ServerConfig` and probed binary capabilities into the actual
  `llama-server` argv. It supports `--jinja`, `--chat-template`, and
  `--chat-template-file` capability detection, but current configuration stores
  only the selected string and extra arguments. It does not hash referenced
  template files or enforce their approval.
- `crates/lmml-state/src/lib.rs::ModelRuntimeProfile` persists model tuning.
  Its `server` field still contains legacy host/port fields, but the TUI now
  overlays the global endpoint. This is unrelated to template admission and
  must not be used as an approval identity.

### Launch

The primary TUI launch path is:

```text
TUI Server tab
  -> crates/lmml-tui/src/event_loop.rs::Action::StartServer
  -> App::server_config
  -> runtime_cli::resolve_model_runtime
  -> App::prepare_server_start
  -> lmml-server::ServerManager::start_with_timeout
  -> health/readiness and served-model verification
```

`crates/lmml-tui/src/runtime_cli.rs` selects upstream or Prism
from GGUF metadata and runtime policy. Prism has strong binary/library/target
attestation, but template approval is not part of that check.

`crates/lmml-server/src/lib.rs` owns the child process, port checks, readiness
polling, model identity verification, and process health monitoring. Its
approved start API also supports a restricted child environment and gateway
ownership; snapshot and approval resolution remain in the TUI admission
command. The ordinary `start_with_timeout` path remains legacy until Slice 6.

### Lease issuance and runtime manifests

`crates/lmml-substrate/src/types.rs` defines `RuntimeManifest` and
`ModelLease`. `crates/lmml-substrate/src/lifecycle.rs` validates and hashes
these records. `crates/lmml-tui/src/lifecycle_runtime_cli.rs` registers runtime
facts and issues leases through the scriptable lifecycle path.

Current runtime manifests bind artifact hash, executable path/hash, command-line
hash, endpoint, context, backend, and capabilities. They do not bind a prompt
configuration hash, template identity, environment allowlist, loaded libraries,
or deployment snapshot.

The lifecycle lease path and ordinary TUI server-start path remain legacy by
default. Under `LMML_APPROVAL_ENFORCEMENT=1`, the TUI is redirected to
`runtime serve-approved-bundle`; this controlled path is the only enforced
launch path until the client migration gate is enabled globally.

### Readiness and request handling

- `lmml-server` verifies health and `/v1/models` identity after launch.
- `crates/lmml-node/src/lib.rs` proxies requests to a configured
  `llama_base_url`; it now forwards the node bearer key upstream, allowing the
  configured URL to be the authenticated LMML gateway. It is not itself the
  approval authority.
- `crates/lmml-router/src/lib.rs` routes requests to configured upstreams; it
  is not currently a template-policy gateway.
- OpenCode can be configured against the TUI-managed endpoint. Until Slice 6
  is enabled, that endpoint may still be the legacy direct server, creating a
  direct backend access path; the approval enforcement switch rejects new
  legacy launches.
- The new `lmml-gateway` validates request policy, rechecks signed admission
  state on every request, and stops streams after revocation. The explicit
  `serve-approved` path uses it. The ordinary TUI and detached runtime-profile
  paths remain legacy unless enforcement is explicitly enabled. Enforced
  `lmml-node` requires the gateway marker, and enforced `lmml-router` requires
  the node marker before routing; OpenCode requires an approved gateway
  endpoint during configuration.

## 3. Identity Contracts

These types belong in `crates/lmml-substrate/src/types.rs` or a private
`approval` module exported by `lmml-substrate`. All schemas require a
version and deterministic serialization.

### Deterministic serialization

Use canonical JSON with:

- fixed field names and schema version;
- lexicographically ordered object keys;
- arrays preserved in semantic order;
- UTF-8 bytes without Unicode normalization;
- exact byte strings represented by lowercase SHA-256;
- explicit representation of absent, empty, disabled, automatic, and default
  values;
- domain-separated hashes such as `LMML/PROMPT-CONFIG/v1` and
  `LMML/RUNTIME-CONFIG/v1`.

Never hash display strings, filesystem paths alone, or a map whose iteration
order is unspecified.

### `TemplateIdentity`

```rust
pub struct TemplateIdentity {
    pub schema_version: u32,
    pub source_template_container_hash: Hash256,
    pub source_template_hash: Option<Hash256>,
    pub selected_variant: String,
    pub builtin_registry_hash: Option<Hash256>,
    pub gguf_embedded_template_hash: Option<Hash256>,
    pub external_template_hash: Option<Hash256>,
    pub effective_template_hash: Hash256,
    pub conversion_status: TemplateConversionStatus,
}
```

`conversion_status` describes only the transformation:
`Preserved`, `Normalized`, or `Changed`. It is never an approval decision.
Any changed bytes produce a new effective identity.

For a named built-in such as `chatml`, `selected_variant` is not sufficient.
`builtin_registry_hash` binds the exact llama.cpp built-in template registry
implementation and build that resolved the name. A change to the registry
invalidates the approval even when the command-line name remains unchanged.

`source_template_container_hash` identifies the source tokenizer/configuration
container from which the source template variant was extracted. It is distinct
from `source_template_hash`, which identifies the selected template bytes.

### `PromptConfiguration`

```rust
pub struct PromptConfiguration {
    pub schema_version: u32,
    pub model_lineage_id: String,
    pub artifact_hash: Hash256,
    pub source_tokenizer_manifest_hash: Hash256,
    pub deployed_tokenizer_metadata_hash: Hash256,
    pub special_tokens_hash: Hash256,
    pub effective_tokenizer_hash: Hash256,
    pub template: TemplateIdentity,
    pub renderer_id: String,
    pub renderer_version: String,
    pub renderer_build_hash: Hash256,
    pub renderer_settings: CanonicalValue,
    pub request_schema_id: String,
    pub request_schema_version: u32,
    pub permitted_template_kwargs: CanonicalSchema,
    pub multimodal_processor_hash: Option<Hash256>,
    pub mmproj_hash: Option<Hash256>,
}
```

The prompt identity covers the model/tokenizer/template/renderer/processor
combination, not the incoming user messages or tool definitions themselves.
Those are request data validated against the permitted schema.

### `RuntimeConfiguration`

```rust
pub struct RuntimeConfiguration {
    pub schema_version: u32,
    pub prompt_configuration_hash: Hash256,
    pub backend: String,
    pub backend_version: String,
    pub executable_hash: Hash256,
    pub resolved_argv: Vec<String>,
    pub allowed_environment: Vec<(String, String)>,
    pub secret_environment: Vec<SecretEnvironmentPolicy>,
    pub loaded_libraries: Vec<LibraryIdentity>,
    pub context_size: u64,
    pub slot_configuration: CanonicalValue,
    pub cache_configuration: CanonicalValue,
    pub backend_target: Option<String>,
}
```

`resolved_argv` is the final argv after all profile defaults, model-family
safeguards, runtime selection, sidecar resolution, and capability translation.
It preserves order, repeated flags, and exact values.

The source tokenizer manifest, deployed tokenizer metadata, reconstructed
special-token map, and effective tokenizer identity are separate because GGUF
conversion may normalize or drop tokenizer/template variants. Admission binds
the deployed and effective identities, not only the source hash.

The child receives an explicit environment allowlist. Secret variables are
represented only by a presence policy and are never stored as values. Variables
outside the allowlist are rejected rather than inherited. The policy must
explicitly classify `PATH`, `HOME`, ROCm/CUDA paths, proxy variables, and
locale variables.

### Deployment manifest

```rust
pub struct DeploymentManifest {
    pub schema_version: u32,
    pub deployment_id: String,
    pub prompt_configuration_hash: Hash256,
    pub runtime_configuration_hash: Hash256,
    pub files: Vec<DeploymentFile>,
    pub executable: FileIdentity,
    pub libraries: Vec<LibraryIdentity>,
    pub environment_hash: Hash256,
    pub created_at: String,
}
```

Every referenced model, tokenizer, template, processor, mmproj, executable,
and library must resolve to a file identity in the deployment. A path is only
metadata; the hash and deployment membership are authoritative.

### Approval receipt

```rust
pub struct TemplateApprovalReceipt {
    pub schema_version: u32,
    pub receipt_id: String,
    pub prompt_configuration_hash: Hash256,
    pub runtime_configuration_hash: Hash256,
    pub deployment_manifest_hash: Hash256,
    pub policy_version: String,
    pub validation_suite_id: String,
    pub decision: ApprovalDecision,
    pub reviewer_id: String,
    pub reviewer_scope: String,
    pub signing_key_id: String,
    pub sequence: u64,
    pub created_at: String,
    pub signature: Signature,
}
```

The first enabled policy uses signatures from the beginning. An unsigned
receipt is not a valid approval. Receipt storage is append-only, protected by
permissions, and includes a monotonic sequence/epoch to prevent restoring an
older valid store after revocation.

### Revocation state

```rust
pub struct RevocationState {
    pub schema_version: u32,
    pub epoch: u64,
    pub revoked_receipts: Vec<RevocationEvent>,
    pub latest_sequence: u64,
    pub signature: Signature,
}
```

Revocation is a signed append-only event referencing the original receipt and
reason. A receipt is invalidated when any bound identity changes, its sequence
is below the protected revocation state, or its signature/policy/suite is no
longer accepted.

### Exact invalidation rules

Invalidate to `NeedsReview` or `Invalidated` when any of these changes:

- source model, derived artifact, tokenizer, special token map, or lineage;
- source, embedded, selected, or external template bytes/variant;
- renderer implementation, build, supported subset, or renderer settings;
- permitted kwargs schema;
- multimodal processor or mmproj;
- final argv, parser/reasoning/template flags, context, slots, cache policy,
  runtime backend, executable, target, library, or allowlisted environment;
- deployment snapshot membership or file hash;
- approval policy, validation suite, signing key, revocation epoch, or receipt
  sequence.

Incoming messages and tool definitions do not invalidate the prompt approval if
they validate against the approved request schema. A new unsupported field is
rejected rather than silently invalidating or widening the approval.

## 4. Deployment Design

### Content-addressed snapshot

Before admission, LMML creates a private deployment staging directory under the
managed data root, for example:

```text
$LMML_DATA_ROOT/lmml/deployments/.staging/<deployment-id>/
$LMML_DATA_ROOT/lmml/deployments/sha256/<deployment-hash>/
```

The published directory contains only content-addressed files and the signed
deployment manifest. Model-sized files should use hard links to immutable
content-addressed payloads where possible; copying is allowed when a separate
filesystem or privilege boundary requires it.

Hard links are not inherently immutable: another hard link can modify the same
inode. Publication therefore requires the source payload to be owned by LMML,
not group/world writable, and opened/read-verified before publication. If
ownership cannot be guaranteed, copy into the private store and hash the copy.

### Atomic publication

1. Resolve all paths and reject symlinks escaping the managed deployment root.
2. Hash every source before staging.
3. Materialize or link files into a unique staging directory.
4. Re-hash staged files and compare identities.
5. Write the manifest and signature with restrictive permissions.
6. `fsync` files and directory where supported.
7. Rename the complete staging directory into the content-addressed final path.
8. Never overwrite an existing deployment hash.

Any failure removes only the private staging directory. Existing deployments
remain untouched.

### Dependency resolution

The resolver must identify:

- GGUF artifact and model lineage;
- tokenizer and special-token assets from the canonical source/derived artifact;
- embedded and external templates;
- image/video processor assets;
- mmproj files;
- exact llama-server executable;
- Prism/upstream backend libraries resolved by `ldd` or an equivalent loader
  inspection;
- runtime target and backend;
- explicit child environment.

Library admission uses a declared loader closure, not a blanket prohibition on
all system libraries. The deployment records required and optional libraries,
permitted system-library roots, permitted accelerator-library roots, and whether
late `dlopen` is allowed. The default is to disallow unlisted late loads. The
post-load process inspection compares mapped library paths and hashes against
that closure; it does not require libc, libstdc++, driver, NSS, or accelerator
libraries to be copied into the deployment when they are explicitly allowed by
the host policy.

Libraries loaded lazily after startup are a specific risk. Startup verification
must inspect the process's mapped files after model load and before readiness is
published. The deployment must use an isolated library search path and reject
unlisted libraries. A later library-load event or changed mapping invalidates
the runtime and stops accepting requests.

### Startup verification

After launch and model readiness, LMML verifies:

- `/proc/<pid>/exe` and command line;
- exact artifact identity from `/v1/models` plus the deployment manifest;
- executable and loaded-library hashes;
- effective endpoint and context/slot properties;
- deployment, prompt, runtime, and receipt hashes;
- no unexpected environment or library mapping.

Only after these checks does LMML publish the endpoint or issue a usable lease.

### Cleanup

Failed starts, cancelled starts, and invalidated deployments cannot remain
reachable. LMML stops the child, waits for exit, removes only unreferenced
staging directories, and retains signed evidence of the failed admission.
Active deployments remain until no lease or process references them.

The approved launch path now performs a post-readiness Linux loader-map check:
every mapped shared library must be declared and hash-match the runtime
configuration, and every required declared library must be observed. The
ordinary TUI launch path remains outside this admission path until migration.

## 5. Admission State Machine

Persisted decisions and computed runtime status are separate.

```text
Unknown
  -> Extracted
  -> NeedsReview
  -> Approved
  -> RuntimeVerified
  -> Revoked

NeedsReview -> Rejected
Approved -> Invalidated
RuntimeVerified -> Invalidated
Revoked -> NeedsReview
```

Persisted states:

- `Unknown`: no trusted extraction or receipt;
- `NeedsReview`: extraction exists, but approval is absent or stale;
- `Approved`: signed human decision for exact prompt/runtime/deployment hashes;
- `Rejected`: signed decision that this configuration must not run;
- `Revoked`: approval was withdrawn by a signed revocation event.

`Blocked` is not a persisted approval decision. It is an operational refusal
returned when a runtime check, request-policy check, signature check, or
approval-store check fails. The external CLI, TUI, receipt, and JSON vocabulary
must preserve this distinction.

Computed states:

- `Extracted`: current metadata exists but has no decision;
- `RuntimeVerified`: a running process matches an approved deployment;
- `Invalidated`: current bytes/configuration no longer match the receipt.

`RuntimeVerified` is never persisted as a human approval. It is recomputed from
the process, deployment, receipt, and revocation state.

The serialized wire values are explicit snake_case strings:

```text
unknown
extracted
needs_review
approved
rejected
revoked
invalidated
runtime_verified
blocked
```

`blocked` is an operational result with a structured reason and is not a
persisted approval decision. TUI labels may use the PascalCase names above, but
JSON, receipts, manifests, and CLI output use only the wire values.

The launch gate fails closed for `Unknown`, `NeedsReview`, `Rejected`,
`Revoked`, and `Invalidated`, returning `Blocked` with a reason. A
receipt-store read error is also a hard failure and returns `Blocked`.

## 6. Gateway Enforcement

### Backend isolation

The first enabled approval path must not expose raw `llama-server` to clients.
LMML should bind the backend to a private Unix socket or loopback-only random
endpoint and place an LMML gateway in front of it. The gateway owns the public
endpoint, authentication, request validation, lease checks, and streaming
lifecycle.

`lmml-node` and `lmml-router` must either call the LMML gateway or receive a
short-lived authenticated lease that proves the upstream deployment. They must
not forward arbitrary client template controls to raw llama-server.

Direct TUI `llama-server` access, direct OpenCode access, and unmanaged
`llama-server` processes remain unsupported until they pass through the same
admission boundary.

### Permitted request fields

The gateway initially permits message content, roles, tool definitions, tool
results, sampling fields already supported by the approved runtime, and a
versioned set of template kwargs. It rejects:

- template replacement or template file paths;
- renderer selection or Jinja controls;
- parser selection not in the receipt;
- unknown kwargs or fields;
- model paths, backend paths, runtime selection, or environment fields.

Add a versioned `RequestPromptSchema` bound by `PromptConfiguration`:

```rust
pub struct RequestPromptSchema {
    pub schema_version: u32,
    pub schema_id: String,
    pub allowed_roles: Vec<String>,
    pub content_kinds: Vec<String>,
    pub tool_schema_limits: CanonicalValue,
    pub multimodal_limits: CanonicalValue,
    pub reasoning_controls: CanonicalSchema,
    pub assistant_prefill: bool,
    pub max_nesting: u32,
    pub max_bytes: u64,
    pub max_items: u64,
    pub max_context_tokens: u64,
}
```

It covers message roles/content forms, tool definitions/results, multimodal
blocks, reasoning controls, assistant prefill, nesting/byte/item limits, and
token/context budgets. Template kwargs are validated against the
`CanonicalSchema` and this request schema. The gateway must render or validate
each request through the approved renderer contract before forwarding it.
Values are constrained by type, enum/range, maximum length, and nesting depth.

### Streaming and concurrency

Each request captures the approval epoch and runtime lease. The gateway:

- checks approval before dispatch;
- reserves a runtime slot using the existing server capacity policy;
- propagates client cancellation to the backend;
- removes the request on disconnect;
- rejects new work after revocation;
- applies the selected cancellation policy to active streams;
- prevents a revoked deployment from re-entering the ready pool.

Recommended default: revoke immediately blocks new requests, asks active
requests to cancel, and force-stops the backend after a short bounded grace
period.

### Approval-store failure

If approval or revocation state cannot be read or verified, the gateway fails
closed with a structured error. It must not use a cached approval indefinitely.
Recommended default: cache only a signed approval for the current epoch and
require a fresh store check at request admission; use a short bounded cache only
for availability, with revocation epoch checks.

## 7. Renderer Verification

### Exact production renderer

The approval renderer must be the exact pinned llama.cpp implementation and
build used by the deployment, including its supported Jinja subset, tokenizer
loading, compile flags, linked libraries, and renderer settings.
Python Transformers is an extraction/comparison tool, never the production
admission oracle when llama.cpp renders the prompt.

The recommended mechanism is a renderer operation exposed by the exact
deployed llama.cpp build. If that is not available, the fallback is a renderer
helper built and hashed as part of the same deployment, with matching source
revision, build flags, tokenizer identity, executable identity, library
closure, and runtime settings. A helper from the same source revision alone is
not sufficient. Approval is disabled when neither mechanism can prove parity.

The acceptance test compares the production/helper build identity, tokenizer
identity, exact rendered bytes, and token IDs for every fixture. Python-only
rendering cannot satisfy admission.

The current implementation uses the first mechanism: `lmml model
verify-template` calls `/apply-template` and `/tokenize` on the exact running
llama-server, verifies `/health` and `/v1/models`, and records the supplied
binary/model hashes with the fixture result. Approved launch additionally
requires `renderer_id = llama.cpp/server-native` and the prompt renderer build
hash to equal the admitted server executable hash. The verifier is not yet
automatically invoked by the TUI review workflow; that remains a Slice 6
integration task.

The gateway checks approval while a response is streaming, including while the
backend is stalled between chunks. Revocation, invalidation, approval-store
failure, and the bounded request deadline terminate the forwarded stream
rather than waiting indefinitely for another backend chunk.

### Fixtures and supported features

Store fixture inputs and expected hashes/token IDs for:

- ordinary system/user/assistant conversations;
- empty and repeated messages;
- Unicode, control characters, and delimiter-like user content;
- tool definitions, calls, and results;
- reasoning enabled and disabled;
- assistant prefill and continuation;
- multimodal content and mmproj;
- each template variant;
- malformed tool schemas;
- very large system prompts;
- permitted and rejected kwargs.

Compare exact rendered bytes, token IDs, special-token boundaries, and output
limits. Any unsupported Jinja call/filter/include, recursion, render timeout,
memory limit, output-size limit, or malformed feature rejects the configuration.

The renderer subprocess receives no network, secrets, inherited sensitive
environment, writable source directory, or ambient file descriptors. It has
bounded CPU time, memory, output size, recursion, and input size.

## 8. Migration

Existing artifacts, model profiles, and running runtimes receive no implicit
approval. The first enabled release treats them as `Unknown` or `NeedsReview`.

Migration behavior:

1. Existing artifact records remain readable and retain their current schema.
2. LMML extracts template/deployment identities on inspection or explicit
   `model template inspect`.
3. Existing profiles with embedded-template defaults may be reviewed as a new
   prompt configuration, but approval is never inferred from the model family,
   filename, old runtime manifest, or prior successful launch.
4. Existing running runtimes continue only in legacy inspection mode until
   restart; they cannot receive a new approved lease.
5. Restarting a runtime requires a deployment snapshot and approval receipt.
6. TUI, JSON, and CLI output show `inspection_only`, `needs_review`, or
   `blocked`, with the exact reason.

No automatic conversion of an old receipt into a signed approval is permitted.

## 9. Implementation Slices

### Slice 0: contracts and threat-model fixtures

Files:

- `crates/lmml-substrate/src/types.rs`
- `crates/lmml-substrate/src/lib.rs`
- new `crates/lmml-substrate/src/approval.rs`
- `docs/template-approval-deployment-plan.md`

Work:

- Add versioned identities, receipt, revocation, canonical serialization, and
  typed errors.
- Select and implement the trust root, key pinning, rotation/backup/recovery
  rules, signed receipt policy, and no unsigned fallback.
- Add the protected revocation epoch/sequence anchor and restore detection.
- Add fixtures for canonicalization and state transitions.

Acceptance:

- identical inputs produce identical domain-separated hashes;
- absent/empty/disabled/automatic values differ deterministically;
- signature, sequence, and revocation rollback are rejected;
- key replacement, unsigned receipts, and old receipt/revocation stores return
  `Blocked` until an explicit signed migration is completed.

Compatibility: additive substrate schemas; existing artifact/runtime records
remain readable. No launch behavior changes.

### Slice 1: extraction and effective configuration

Files:

- `crates/lmml-tui/src/model_cli.rs`
- `crates/lmml-tui/src/model_cli/derive.rs`
- `crates/lmml-tui/src/model_cli/derive/admission.rs`
- `crates/lmml-substrate/src/lib.rs` importer functions
- `crates/lmml-compat/src/lib.rs`
- `crates/lmml-tui/src/app.rs`

Work:

- Extract source tokenizer/template variants and GGUF embedded metadata.
- Resolve external template files and mmproj/processor assets.
- Add a function that returns the final `ServerConfig` and final `build_argv`
  result after all safeguards and runtime selection.
- Produce `PromptConfiguration` and `RuntimeConfiguration` without launching.

Acceptance:

- source/deployed/effective template identities are distinct;
- changing `--chat-template-file`, `--jinja`, parser flags, or mmproj changes
  the corresponding hash;
- no path-only identity is accepted.

Compatibility: existing import/derive commands remain unchanged unless an
explicit inspection command requests template metadata.

### Slice 2: immutable deployment snapshots

Files:

- new `crates/lmml-substrate/src/deployment.rs`
- `crates/lmml-tui/src/model_cli/derive.rs`
- `crates/lmml-tui/src/runtime_cli.rs`
- `crates/lmml-build/src/verification.rs`

Work:

- Implement staging, hash verification, atomic publication, ownership checks,
  library resolution, and cleanup.
- Add explicit child environment construction.
- Verify loaded libraries after model load, including lazy loads before readiness.

Acceptance:

- substitution between staging and launch fails;
- mutable hard-link sources are rejected or copied;
- libraries outside the declared loader closure and environment overrides fail;
- failed/cancelled starts leave no reachable partial deployment.

Compatibility: deployment snapshots are opt-in until Slice 4.

### Slice 3: renderer and compatibility suite

Files:

- new `crates/lmml-substrate/src/renderer.rs`
- new `crates/lmml-tui/src/model_cli/template.rs`
- `crates/lmml-compat/src/lib.rs`
- `docs/fixtures/` for prompt/token fixtures

Work:

- Add exact pinned llama.cpp renderer invocation.
- Enforce renderer resource limits and unsupported-feature rejection.
- Store byte/token-ID fixture results and validation-suite identity.

Acceptance:

- production renderer output matches fixtures;
- Python-only output cannot satisfy admission;
- unsupported templates, kwargs, multimodal inputs, and parser controls fail.

Compatibility: inspection can report findings; no runtime is admitted yet.

### Slice 4: deployment admission and lease planning (no serving)

Files:

- `crates/lmml-tui/src/event_loop.rs`
- `crates/lmml-server/src/lib.rs`
- `crates/lmml-tui/src/runtime_cli.rs`
- `crates/lmml-tui/src/lifecycle_runtime_cli.rs`
- `crates/lmml-substrate/src/lifecycle.rs`
- `crates/lmml-substrate/src/types.rs`

Work:

- Resolve and validate signed approval, deployment manifest, and current
  revocation state before a deployment can be marked eligible.
- Bind the ordinary TUI launch plan and scriptable runtime lease plan to the
  same admission function, but do not start a serving backend or issue a
  usable lease.
- Implement the process/executable/library verification contract as a private
  test-only admission operation; no public endpoint or raw backend transport is
  created in this slice.
- Persist only computed `Invalidated`/inspection evidence, not a new approval.

Acceptance:

- unapproved, changed, revoked, or stale configurations cannot become eligible;
- direct process substitution and library substitution fail in the private
  verification operation;
- receipt-store failures fail closed;
- no endpoint, lease, or readiness signal is advertised from this slice.

Compatibility: this remains `inspection_only`; existing models require
explicit review after migration. Enforced approval is not enabled here.

### Slice 5: gateway and backend isolation

Files:

- new `crates/lmml-gateway/` workspace crate, or the selected equivalent
  gateway module;
- `Cargo.toml`
- `crates/lmml-node/src/lib.rs`
- `crates/lmml-router/src/lib.rs`
- `crates/lmml-server/src/lib.rs`
- `crates/lmml-api/`

Work:

- Put the public endpoint behind the LMML gateway.
- Keep raw backend access private and authenticated.
- Validate request fields, kwargs, tool schemas, limits, and lease identity.
- Recheck approval/revocation on each request and enforce cancellation.
- Fail closed on approval-store errors.
- Keep the feature in `inspection_only`; Slice 5 tests must not enable
  enforced approval for legacy clients.

Acceptance:

- direct backend calls are refused or unreachable;
- request-level template/parser overrides are rejected;
- unknown kwargs and over-limit requests fail;
- concurrent revocation blocks new streams and applies bounded cancellation
  to active streams;
- node/router paths cannot bypass the gateway.

When enforcement is enabled, an `lmml-node` health response carries
`x-lmml-approved-upstream: approved` only after the node has verified its
configured inference upstream as an LMML gateway. `lmml-router` requires that
marker before routing to any node, so the LAN path cannot silently downgrade
from an approved gateway to a raw llama-server endpoint. Legacy mode does not
require the marker.

The gateway and private backend transport must pass their tests, but Slice 5
does not enable enforced approval for existing clients.

Compatibility: clients remain in inspection-only/legacy mode until Slice 6
migrates them to the gateway.

`lmml-gateway::ApprovalStore` now reloads and verifies the signed receipt,
trust root, and optional revocation state on each request. Store read,
signature, identity, or revocation failures become `blocked`; the gateway
does not retain an indefinitely valid in-memory approval.

### Slice 6: client migration, inspection, review, and TUI

Files:

- `crates/lmml-tui/src/model_cli.rs`
- `crates/lmml-tui/src/event_loop.rs`
- `crates/lmml-tui/src/main.rs`
- `crates/lmml-tui/src/lifecycle_runtime_cli.rs`
- `docs/how-to-use.md`
- `docs/llm-model-support.md`

Work:

- Add machine-readable inspect/review/status commands.
- Display exact receipt, identity, rejection, and revocation reasons.
- Make legacy artifacts visibly inspection-only.
- Document the direct-backend restriction and migration flow.
- Migrate TUI, `lmml-node`, `lmml-router`, OpenCode, and other documented
  clients to the gateway endpoint.
- Reject or clearly mark legacy direct-backend configurations rather than
  silently treating them as approved.
- Enable enforced approval only after all required clients have migrated and
  the Slice 6 integration scenarios pass.

Acceptance: operators can inspect exact diffs and approve/reject without
guessing which template or runtime is active. Only after this slice may the
release switch from inspection-only to enforced approval.

### Slice 7: operational hardening

Files:

- `crates/lmml-substrate/src/approval.rs`
- managed key/receipt storage modules;
- `docs/release-checklist.md`;
- integration tests.

Work:

- Add operational key rotation tooling, backup/restore workflows, and recovery
  drills for the Slice 0 trust root.
- Add platform-specific OS keyring/TPM integration where available.
- Improve audit export, monitoring, and incident recovery without weakening the
  Slice 0 signature or rollback rules.

Acceptance: operational recovery preserves the Slice 0 guarantees; an old
receipt store, unsigned receipt, wrong key, or revoked sequence cannot
re-enable a deployment.

## 10. Completion Evidence

The implementation is complete only after these commands and scenarios pass:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Required integration scenarios:

1. Import a source tokenizer with two template variants and verify distinct
   identities.
2. Convert to GGUF with preserved, normalized, and changed templates; verify
   only explicitly reviewed configurations can proceed.
3. Substitute a template, tokenizer, model, mmproj, executable, or library
   after approval; launch must fail.
4. Modify an allowlisted environment variable; launch must fail.
5. Attempt to replace an inode through a hard link; deployment must fail or use
   the protected copy.
6. Attempt a direct raw backend request; it must be unreachable or rejected.
7. Submit template replacement, parser controls, unknown kwargs, malformed
   tools, oversized prompts, and unsupported multimodal content; all must fail
   closed.
8. Start approval and launch concurrently; only one consistent deployment may
   become reachable.
9. Revoke during approval, during launch, and during an active stream; new
   requests must fail and active work must follow the bounded cancellation
   policy.
10. Restore an older receipt store; monotonic revocation state must reject it.
    Restore an older receipt store together with an older revocation state;
    the protected external epoch/key anchor must still reject both.
11. Restart LMML and recover an approved deployment only after recomputing all
   identities and process/library state.
12. Verify an approved request through the TUI, node, router, and OpenCode
   paths, with one runtime manifest and one lease identity.

Each scenario must emit machine-readable evidence containing deployment hash,
prompt configuration hash, runtime configuration hash, receipt ID/sequence,
revocation epoch, process ID, endpoint, and rejection reason where applicable.

Current validation evidence: `cargo test --workspace --offline` passes outside
the restricted socket sandbox, including node/router HTTP integration and TUI
end-to-end tests; `cargo clippy --workspace --all-targets --offline --
-D warnings`, `cargo fmt --all -- --check`, and `git diff --check` also pass.

## Unresolved Decisions and Recommended Defaults

| Decision | Recommended default | Reason |
|---|---|---|
| Receipt trust | Signatures required from first enabled path | Avoid ambiguous unsigned security boundary |
| Key ownership | Operator-provisioned key with pinned public key; OS keyring/TPM preferred | A local generated key is only valid if same-user compromise is accepted |
| Key rotation | Explicit signed migration with old/new key IDs and protected epoch | Prevent silent key replacement and receipt rollback |
| Receipt rollback anchor | Separate protected epoch file or OS keyring, with journal fallback | A sequence stored only beside receipts can be restored with them |
| Backend access | Private Unix socket or loopback random port behind gateway | Prevent direct policy bypass |
| Request approval cache | Short bounded cache with signed epoch check | Balances latency and revocation freshness |
| Revocation behavior | Block new requests immediately; cancel active work; stop runtime after bounded grace | Prevents reuse of revoked deployments |
| Template overrides | Reject request-level overrides in v1 | Simplest fail-closed policy |
| Renderer | Renderer operation from exact deployed build; helper only with matching deployment identity | Avoid Python/llama.cpp and helper/server divergence |
| Approval enablement | Inspection-only until Slice 6 client migration and integration tests pass | Prevent approved raw-backend bypass |
| Existing models | Inspection-only until explicitly reviewed | No implicit approval |
| Deployment storage | Content-addressed LMML-managed store, hard links only for protected immutable sources | Avoid substitution races without duplicating all weights |
| Gateway placement | New workspace crate unless an existing API crate can own lifecycle and streaming cleanly | Keep request policy separate from raw process management |

## Rollback Procedure

Before enabling each slice, retain the previous binary and state schema. If
admission or gateway behavior fails:

1. stop the managed runtime;
2. mark the deployment invalidated and retain evidence;
3. do not delete artifacts or receipts;
4. restore the previous LMML binary/configuration only in explicit legacy
   inspection mode;
5. keep approved endpoints unavailable until the failed policy state is
   diagnosed;
6. never restore an older receipt/revocation store without verifying its
   sequence and epoch against the protected current state.

Rollback restores availability of the management tool, not approval of an
unverified runtime.
