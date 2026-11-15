# Technical design 01: one protocol for every model

| | |
|---|---|
| Status | Draft for review. **Nothing described here is implemented.** |
| Date | 2026-11-15 |
| Repository | commit `fddacce`. This document adds only the folder `docs/design/`. |
| How it was written | Read-only measurements of the existing program on the development sandbox (2 virtual CPUs, 3.8 GiB, Linux x86-64), and reading of provider documentation. No code, test or workflow was changed. No model experiment was run. |
| What was not possible | Calling any provider's API. I have no API keys and the sandbox cannot reach the providers. Every statement about a provider's message format therefore comes from its documentation (Appendix A), not from a test. |

Files in this folder:

- `01-universal-interface.md`: this document.
- `vlp-0.1-draft.schema.json`: the proposed request and response schemas (JSON Schema 2020-12).
- `check_examples.py`: checks every JSON example in this document against that schema.

## 1. Summary

The interface is defined once, as a versioned catalogue of seven operations with a JSON Schema for what goes into each and what comes out. I call it the VL Protocol. The command line, a local server on standard input and output, an HTTP API, the tool definitions for each provider's function-calling format, an optional MCP server and the agent skill are either generated from that catalogue or are thin mappings onto it. A test replays the same requests through every one of them and requires the same diagnostics and the same picture, bit for bit, as the engine called directly.

Five decisions carry most of the design:

1. **Operations keep no hidden state.** Every call carries the program text. A rendered picture gets an identifier computed from the program, the options and the renderer revision, so asking twice gives the same identifier and the same bytes, and a repeated call never does the work twice.
2. **Parameters are flat.** A request contains no nested objects. The arguments of a model's tool call are therefore the protocol parameters, unchanged, and an adapter has nothing to translate on the way in.
3. **The language reference is generated from the tables the compiler executes and is served by the engine itself.** Whatever a model reads through the `reference` operation describes exactly the engine version that will run its program.
4. **One picture, three forms.** The file itself; a view with a number drawn on every object, for models that read images; and a grid of letters with a colour legend, for models that do not. All three come from the pixels that were actually rendered.
5. **Provider differences are data.** The one difference that matters is where a picture may be placed in a conversation. A small capability profile supplied by the integrator decides it. No adapter contains a table of model names.

Three things the investigation found that you should know before reading further:

- **The existing program can be crashed or stalled by short inputs** (section 3.3). A 20-kilobyte program aborts the process with a stack overflow. A 95-byte program keeps `vl check` busy for 20 seconds. Nothing can stop a render once it starts. None of this matters much for one person at a command line. All of it matters the moment the engine sits behind a server or an agent, so the security design in section 11 starts from these facts.
- **Today's feedback describes a stand-in, not the delivered picture.** The inspection that produces the colours and positions runs its own render at 256 pixels. The design measures the delivered picture instead (section 10.4).
- **My own notes on the language were incomplete.** Block comments, semicolons as separators and quoted names are accepted by the compiler and appear in neither reference file. I learned of two of them only by trying them today. This is the strongest argument for generating the reference from the code rather than writing it by hand.

The decisions I need from you are collected in section 18.

## 2. Requirements and where each is answered

| Requirement | Answer | Section |
|---|---|---|
| A versioned, model-independent protocol is the single normative interface | Operation catalogue with schemas; everything else derives from it | 4, 5, 7 |
| OpenAI-compatible tool and function calling | Generated definitions in both OpenAI formats (Chat Completions and Responses) | 9.3 |
| Gemini-compatible tool integration | Generated definitions for both Gemini APIs | 9.3 |
| Claude-compatible tool integration | Generated definitions for the Messages API | 9.3 |
| Generic HTTP API | HTTP binding with an OpenAPI description generated from the catalogue | 4.5 |
| Local model integrations | The OpenAI chat format covers local servers; schema- and grammar-constrained output for models without tool calling; everything works offline | 9.2, 9.6 |
| CLI for agents with shell access | Existing commands kept; protocol-exact `vl call`; the agent skill | 4.5, 12 |
| Optional MCP adapter, not a mandatory dependency | Separate crate behind its own subcommand; nothing else depends on it | 9.7 |
| No hardcoded provider or model | Capability profile is data; a test forbids provider names in engine and protocol crates | 9.4, 4.1 |
| Complete language specification, machine- and human-readable, matching the implementation | One registry in the compiler; both forms generated from it; tests fail on any difference | 6 |
| AI visual feedback loop with the real picture and structured diagnostics | `check`, `render`, `view`, `measure`; the loop mapped step by step | 4.6, 10 |
| No provider SDK in the rendering core; core separate from adapters | Layering with a dependency rule that tests enforce | 4.1 |
| Adapters must not change rendering semantics | Adapters cannot touch the program; identical pixel hash required through every path | 9.1, 13 |
| Not simply a wrapper around the current renderer | The protocol carries no knowledge of the current vocabulary; engine growth reaches every model without adapter changes | 15 |

## 3. Starting point

### 3.1 What exists (verified today)

| Fact | Evidence |
|---|---|
| Six Rust crates, 14,771 source lines, no third-party crates, `#![forbid(unsafe_code)]` in each | `Cargo.toml`, line count |
| The five engine crates never touch files, the network, processes, environment variables or standard input and output. Only `vl-cli` does. | search for `std::fs`, `std::net`, `std::process`, `std::env`, `std::io` |
| The only entry point is the command line: `vl check`, `inspect`, `render`, `version`. Its JSON output has no version and no schema. | `crates/vl-cli/src/main.rs` |
| The same program, options and seed give a byte-identical PNG with one thread and with two | measured on 14 November; not yet checked across operating systems or processor types |
| Automatic exposure is the same at every size: EV 13.64 for the fruit bowl from 300 to 1568 pixels wide | measured today |
| Each sample has its own random stream, keyed by seed, pixel and sample number | `integrator.rs`, `render_pixel` |
| The integrator can render a rectangular part of a picture and reports progress through a callback | `RenderOpts.region`, `RenderOpts.progress`; the command line does not offer the region |
| The integrator already produces albedo, normal, depth and noise-variance buffers and an exact object-per-pixel buffer | `Frame`, `id_pass` |
| A JSON parser and writer exist in `vl-core`, with a nesting limit of 128 | `json.rs` |
| `check --json` already reports each object's centre and size in metres after relations such as `on:` are resolved | output of today's runs |
| The release profile uses `panic = "unwind"`, so a panic can be caught at a boundary | `Cargo.toml` |

### 3.2 What it costs (fruit bowl example, 2 virtual CPUs)

| Call | Time | PNG size |
|---|---|---|
| `check` | 0.004 s | |
| `inspect` (its own 256-pixel render with 24 samples) | 1.7 s | |
| render 512 × 288, 16 samples | 3.9 s | 142 KB |
| render 1024 × 576, 32 samples | 24.4 s | 437 KB |
| render 1568 × 882, 32 samples | 56.3 s | 914 KB |
| render 960 × 540, 96 samples (the example's own settings) | 68.5 s | |
| render 3840 × 2160, 64 samples (still life, measured 14 November) | 998 s; peak memory 874 MB | 6.7 MB |

Two consequences shape the design. Only a small preview fits inside the few seconds an interactive tool call can reasonably take. A full picture needs minutes and therefore a job that can be waited for, asked about and cancelled.

### 3.3 Defects found while measuring

All measured today on commit `fddacce`. Every input is within the limits the program enforces.

| Input | Size | Effect |
|---|---|---|
| `sphere s { radius: ` followed by 20,000 minus signs and `1 }` | 20 KB | The process aborts: "thread 'main' has overflowed its stack", exit status 134. 5,000 minus signs are handled. The nesting limit covers brackets, parentheses and blocks, but not prefix operators. |
| A sum of 200,000 terms, `0.000001+0.000001+…` | 1.8 MB | Same abort |
| A loop that creates 20,000 spheres (Appendix B) | 95 bytes | `check` takes 19.6 s. 5,000 objects take 0.84 s and 10,000 take 3.3 s: the time grows with the square of the object count. The likely cause is the pairwise loop in `semantic_checks`; I did not profile it. The object limit is 500,000. |
| One block with 200,000 unknown properties | 3 MB | No result within 30 s. 20,000 take 0.40 s. |
| Any large render | | Nothing can cancel or time-limit it. The option bounds allow 16384 × 16384 pixels; at the measured 105 bytes per pixel that would need about 28 GB. I did not run it. |
| A file that starts with a byte-order mark | 3 bytes | Rejected with E100. Some editors on Windows add this mark. |
| An identifier of 100,000 characters | 100 KB | Accepted and echoed back in full. One of 3 MB is accepted as well. |

A stack overflow is not a Rust panic. It cannot be caught, and it ends the whole process. These probes took a few minutes, so a systematic search will find more.

### 3.4 Gaps this design has to close

- The reference documents 20 of the 62 message codes. 26 of about 80 property names appear in neither reference file. No example uses a unit although 13 unit suffixes are implemented.
- The trimmed feedback hides totals. With one error present the output says `"warnings": 0` even when there are warnings. Nine warnings of one kind are reported as a count of four.
- The skill's description speaks of "the bundled `vl` command". The folder contains two text files and no program. That sentence is my error.
- No licence file exists, although `Cargo.toml` and the skill both declare `MIT OR Apache-2.0`.

## 4. Protocol architecture

### 4.1 Layers

```
 models and agents
     |   provider formats: OpenAI chat, OpenAI responses, Anthropic, Gemini, Ollama, MCP, plain chat, shell
     v
 ADAPTERS              data:  generated tool definitions, capability profiles, reference cards
 (outside the core)    glue:  provider call -> protocol request;  protocol result -> provider message
     |   protocol requests and responses (JSON)
     v
 BINDINGS              command line | JSON lines on stdin and stdout | HTTP | MCP (optional) | Rust library
     |   one function:  Request -> Response
     v
 PROTOCOL LAYER        operation catalogue and schemas, validation, job manager, picture store, limits
     |   Rust calls
     v
 ENGINE (exists)       vl-lang (compiler), vl-render (path tracer, denoiser, inspection), vl-image, vl-ir, vl-core
```

The dependency rule: each layer may use only the layers below it. Three tests enforce it.

- No engine crate depends on a protocol, binding or adapter crate, and no crate depends on a provider SDK. The test reads the manifests.
- `std::net` appears only in the server crate. `std::fs` and `std::process` appear only in the command line, the server and the picture store.
- The words `openai`, `anthropic`, `gemini`, `ollama` and `mcp` do not appear in the source of the engine or protocol crates.

### 4.2 Operations

| Operation | What it does | Typical time | Changes anything? |
|---|---|---|---|
| `capabilities` | Versions, optional features, limits, picture formats, quality presets | instant | no |
| `reference` | The language specification: the short card, a section, one kind, one property, one function, one preset, one message code; as Markdown or as JSON | instant | no |
| `check` | Compiles a program. Returns every diagnostic, where each object ended up, and a cost estimate | milliseconds | no |
| `render` | Compiles and renders. Returns the picture's identifier and a report measured from the picture; or a job when it takes longer than the caller is willing to wait | seconds to minutes | stores a picture |
| `job` | Waits for, asks about or cancels a running render | instant, or the wait asked for | may cancel |
| `view` | A derived view of a stored picture: scaled, cropped to an object, with marks, another channel, or as a text grid | instant | stores a view |
| `measure` | Numbers about a stored picture: colours at points, statistics per object, the difference between two pictures | instant | no |

Today's `vl inspect` corresponds to `render` at the lowest quality with only the report requested. The command stays.

Names reserved for later, each behind a feature name in `capabilities`: `asset` (textures, environment maps, meshes, reference photographs), `edit` (image editing) and `sequence` (animation). Section 15 explains why the first version does not need them and why adding them does not disturb it.

### 4.3 No hidden state, explicit handles

Every operation is complete in itself. `check` and `render` take the whole program text. The only things that outlive a call are stored pictures and running jobs, and both are named by explicit identifiers that the caller passes back as ordinary parameters.

A picture's identifier is computed from the program text, the effective options and the render revision (section 7). Because rendering is deterministic, this has three useful effects:

- Sending the same `render` request twice returns the same identifier and never renders twice. If the first render is still running, the second call joins it. A model that repeats itself costs nothing, and a retry after a network failure is safe.
- An expired picture is not a dead end. `view` answers `not_found` with the hint to render again, and the new picture is identical.
- Two parties can verify they are talking about the same picture by comparing identifiers.

Determinism holds today on one machine and one build. Whether it holds across operating systems and processors is not yet checked (section 13, layer 2). Until it is, an identifier is promised to be stable only on the machine that produced it.

Pictures are kept for a stated time (24 hours by default) within a stated size budget. The retention time is written into the tool descriptions so that a model sees it.

### 4.4 Short and long renders

`render` takes `wait_ms`: how long the caller is willing to block. If the picture is ready in time, the result has status `done`. Otherwise it has status `running` and a job with progress, an estimate of the remaining time and a suggested polling interval. The render continues. `job` waits further, reports, or cancels.

This one mechanism serves every binding. A shell agent with a 30-second command limit, an HTTP client behind a proxy with a 60-second timeout and a tool call that should not stall a conversation all use it the same way. Where a provider has its own facility for slow tools (OpenAI's asynchronous tools [A4], MCP's Tasks extension [A13]) an adapter may use it as well, but nothing depends on it.

Cancellation and time limits need changes in the engine, which has neither today (section 11.3).

### 4.5 Bindings

| Binding | Shape | For whom |
|---|---|---|
| Rust library | `handle(request) -> response` with typed structures | programs that embed the engine |
| Command line, existing | `vl check`, `inspect`, `render`, `version`: unchanged, including today's `--json` output | people; scripts already written |
| Command line, new | `vl call OPERATION` reads parameters as JSON from a file or standard input and prints the response; `vl reference`, `vl view`, `vl job`, `vl render --detach`, `vl tools`, `vl version --json` | agents with a shell |
| JSON lines | `vl serve --stdio`: one request per line in, one response per line out, over a long-lived child process | any programming language; no network involved |
| HTTP | `POST /v1/{operation}` with the parameters as the body; `GET /v1/images/{id}` and `GET /v1/images/{id}/view?...` return bytes; `GET /v1/jobs/{id}`, `DELETE /v1/jobs/{id}`; `GET /v1/reference/{topic}` returns Markdown directly; `GET /v1/openapi.json`; `GET /v1/tools?dialect=…&profile=…` | remote and multi-language use; any model with a URL-fetching tool can read the reference |
| MCP, optional | Tools are the operations; resources are reference sections and pictures | MCP hosts |

`vl call` reads its parameters from a file or standard input rather than from the command line, so an agent never has to quote a multi-line program for a shell.

`vl tools --dialect D --profile P` prints the tool definitions for a provider format. An application developer gets them from the same binary that will execute the calls, so they cannot be out of step with it. The generator is an adapter crate of its own. The command-line program links it; the engine and protocol crates do not.

### 4.6 Your feedback loop, step by step

| Step | Operation | A model that reads images receives | A text-only model receives |
|---|---|---|---|
| Description to scene program | `reference` | the short card in its instructions, sections on request | the same |
| Validation | `check` | diagnostics with line, hint and exact replacement; resolved positions and sizes | the same |
| Rendering | `render`, quality `preview` | picture identifier, report, and the preview attached as an image | picture identifier and report |
| Image inspection | `view`, `measure` | the picture, crops around objects, numbered marks, the object map | the report, the letter grid, colours at chosen points |
| Corrections | `check` and `render` again; `measure` can compare two pictures | | |
| Final rendering | `render`, quality `final`, then `job` | a scaled-down view; the full file goes to the user | the report; the full file goes to the user |

## 5. Request and response schemas

The complete draft is in `vlp-0.1-draft.schema.json`. Every example below is checked against it by `check_examples.py`: the eleven examples validate, and six deliberately wrong messages are rejected. The content of the examples comes from real runs of today's program wherever today's program can produce it; members that do not exist yet are marked.

### 5.1 Conventions

- JSON in UTF-8. Member names are lower case with underscores.
- A number with a unit carries the unit in its name: `_m`, `_s`, `_ms`, `_percent`, `_bytes`, `_ev`. Today's output already follows this in most places.
- Positions in a picture are fractions of its width and height from the top-left corner, so they mean the same in a preview and in the final picture. Wherever possible a caller names an object instead of giving coordinates.
- Positions in the program are a line and a column, both starting at 1 and counting characters (this is what the compiler reports today; verified with non-ASCII text), plus an offset and length in UTF-8 bytes for programs that apply fixes.
- Numbers are finite. No timestamps appear in results, because results must be reproducible.
- A client ignores result members it does not know. A server rejects request members it does not know and names the accepted ones, because a model that invents a parameter learns more from that message than from silence. Members that start with `x_` are exempt.

### 5.2 Envelope

A request names the protocol version it was written for, an optional identifier that is echoed back, the operation and its parameters. A response says whether the call itself worked and which engine answered.

<!-- schema: Request -->
```json
{
  "vlp": "0.1", "id": "7", "op": "check",
  "params": {
    "program": "plane ground { }\nbox table { size: [1.2, 0.75, 0.8], on: ground }\nsphere apple { radus: 0.04, at: [0, 2, 0], color: red }\n",
    "diagnostics": "all"
  }
}
```

<!-- schema: Response -->
```json
{
  "vlp": "0.1", "id": "7", "op": "check", "ok": true,
  "engine": {"name": "vl", "version": "0.2.0", "render_revision": 1, "language": "1.0"},
  "result": {
    "valid": true, "counts": {"errors": 0, "warnings": 2, "findings": 0}, "diagnostics_omitted": 0,
    "diagnostics": [
      {
        "code": "W320", "severity": "warning",
        "message": "'apple' floats 1.21 m above 'table' (nothing touches its underside)",
        "hint": "to rest it there write  on: table  - ignore this if it is meant to hang in the air",
        "line": 3, "col": 1, "offset": 66, "length": 12,
        "source_line": "sphere apple { radus: 0.04, at: [0, 2, 0], color: red }", "object": "apple",
        "data": {"gap_m": 1.21, "above": "table"}
      },
      {
        "code": "W345", "severity": "warning",
        "message": "'sphere' has no property 'radus'; it was read as 'radius'", "hint": "write  radius",
        "line": 3, "col": 16, "offset": 81, "length": 5,
        "source_line": "sphere apple { radus: 0.04, at: [0, 2, 0], color: red }", "object": "apple",
        "fix": {"offset": 81, "length": 5, "replace_with": "radius"}
      }
    ],
    "objects": [
      {"name": "ground", "kind": "plane", "line": 1, "material": "default"},
      {
        "name": "table", "kind": "box", "line": 2, "material": "default", "center_m": [0, 0.375, 0],
        "size_m": [1.2, 0.75, 0.8], "rests_on": "ground"
      },
      {
        "name": "apple", "kind": "sphere", "line": 3, "material": "apple.color", "center_m": [0, 2, 0],
        "size_m": [0.08, 0.08, 0.08]
      }
    ],
    "program": {
      "bytes": 122, "lines": 3,
      "sha256": "05e8b9cd1241317caffd66b2ff73a6b7fb16651194ee1e109c28842b95883c89", "language": "1.0",
      "normalized": []
    }
  }
}
```

Compared with today's `check --json`, the new members are `counts` (totals before any trimming), `diagnostics_omitted`, `offset` and `length`, `object`, `data`, `fix`, `rests_on`, `program`, and the envelope itself. `center_m` and `size_m` exist today as `center` and `size`.

`fix` is a replacement a program can apply without understanding the message. `data` carries the numbers that the message text contains, under names listed per code in the code registry (section 6.2).

### 5.3 Rendering

<!-- schema: Request -->
```json
{
  "vlp": "0.1", "id": "8", "op": "render",
  "params": {"program": "(the 19 lines of examples/fruit_bowl.vl)", "quality": "preview", "wait_ms": 20000}
}
```

<!-- schema: Response -->
```json
{
  "vlp": "0.1", "id": "8", "op": "render", "ok": true,
  "engine": {"name": "vl", "version": "0.2.0", "render_revision": 1, "language": "1.0"},
  "result": {
    "status": "done", "valid": true, "counts": {"errors": 0, "warnings": 0, "findings": 0},
    "diagnostics": [], "diagnostics_omitted": 0,
    "image": {
      "id": "img_89bc392c3a90213be3e450110f52eb22", "width": 512, "height": 288, "samples": 16,
      "seed": 1, "quality": "preview", "denoised": true, "exposure_ev": 13.64, "format": "png",
      "bytes": 141958, "sha256": "6ea4c71fdcf79682bafd93d5e7c68a5b2f815ae5c990d5baec39ebcdfc462b01",
      "pixel_hash": "e069b11a7f2d10d9", "render_ms": 3920, "million_rays": 7.94,
      "path": "/home/user/.cache/vl/images/img_89bc392c3a90213be3e450110f52eb22.png",
      "expires_in_s": 86400
    },
    "report": {
      "summary": {
        "mean_brightness_percent": 25, "black_percent": 2.6, "white_percent": 0, "background_percent": 0,
        "noise_percent": 1.8
      },
      "objects": [
        {
          "name": "counter", "kind": "box", "line": 6, "mark": 1, "in_frame": true,
          "image_percent": 60.91, "visible_percent": 94,
          "box": {"left": 0, "top": 0.26, "right": 1, "bottom": 1}, "cut_off": true, "distance_m": 1.33,
          "color": {"hex": "#8a552c", "name": "dark orange (brown)"}
        },
        {
          "name": "bowl", "kind": "cup", "line": 7, "mark": 2, "in_frame": true, "image_percent": 1.81,
          "visible_percent": 56, "box": {"left": 0.28, "top": 0.31, "right": 0.5, "bottom": 0.53},
          "cut_off": false, "distance_m": 1.06, "color": {"hex": "#afa099", "name": "medium gray"},
          "hidden_by": ["orange_4", "orange_2", "orange_3"]
        },
        {
          "name": "lemon", "kind": "sphere", "line": 10, "mark": 8, "in_frame": true,
          "image_percent": 0.12, "visible_percent": 52,
          "box": {"left": 0.39, "top": 0.36, "right": 0.43, "bottom": 0.41}, "cut_off": false,
          "distance_m": 1.11, "color": {"hex": "#b69d36", "name": "medium yellow"},
          "hidden_by": ["orange_2", "orange", "orange_3"]
        }
      ],
      "findings": []
    }
  }
}
```

The numbers are from a real run except `noise_percent`, `mark`, `line`, `path`, `expires_in_s` and the identifier, which do not exist yet. Only three of the seventeen objects are shown.

Quality is a rule for size and sample count, defined once in the protocol so that it means the same through every binding:

| Quality | Longest side | Samples | Measured on the fruit bowl |
|---|---|---|---|
| `draft` | 256 | 8 | about 1 s (1.3 s at 320 × 180) |
| `preview` (default) | 512 | 16 | 3.9 s |
| `standard` | 1024 | 32 | 24.4 s |
| `final` | as the program says (1280 × 720 and 64 samples if it says nothing) | as the program says | 68.5 s at 960 × 540, 96 samples |

The aspect ratio always comes from the program. Explicit `width`, `height` or `samples` in a request override the quality rule. The exposure does not depend on the quality (section 3.1), so a preview is as bright as the final picture.

When the picture is not ready within `wait_ms`:

<!-- schema: Response -->
```json
{
  "vlp": "0.1", "id": "9", "op": "render", "ok": true,
  "engine": {"name": "vl", "version": "0.2.0", "render_revision": 1, "language": "1.0"},
  "result": {
    "status": "running", "valid": true, "counts": {"errors": 0, "warnings": 0, "findings": 0},
    "diagnostics": [], "diagnostics_omitted": 0,
    "job": {
      "id": "job_3adec89ad2e90180c2b374f8ccfed1ed", "state": "running", "progress": 0.29,
      "elapsed_ms": 20000, "eta_ms": 49000, "poll_after_ms": 5000
    },
    "estimate": {
      "width": 960, "height": 540, "samples": 96, "seconds": 69, "memory_bytes": 55000000,
      "basis": "measured"
    }
  }
}
```

When the program has errors, nothing is rendered. This is a successful call with an unsuccessful program:

<!-- schema: Response -->
```json
{
  "vlp": "0.1", "id": "10", "op": "render", "ok": true,
  "engine": {"name": "vl", "version": "0.2.0", "render_revision": 1, "language": "1.0"},
  "result": {
    "status": "rejected", "valid": false, "counts": {"errors": 1, "warnings": 0, "findings": 0},
    "diagnostics_omitted": 0,
    "diagnostics": [
      {
        "code": "E201", "severity": "error", "message": "unknown name 'tabel'",
        "hint": "nothing called 'tabel' exists above this line. Use the name of an object created earlier, or a number",
        "line": 2, "col": 34, "offset": 50, "length": 5,
        "source_line": "sphere apple { radius: 0.04, on: tabel, color: red }"
      }
    ]
  }
}
```

### 5.4 Views

<!-- schema: Request -->
```json
{
  "vlp": "0.1", "id": "13", "op": "view",
  "params": {
    "image": "img_89bc392c3a90213be3e450110f52eb22", "crop_object": "bowl", "crop_margin": 0.3,
    "max_side": 512, "annotate": "marks"
  }
}
```

<!-- schema: Response -->
```json
{
  "vlp": "0.1", "id": "13", "op": "view", "ok": true,
  "engine": {"name": "vl", "version": "0.2.0", "render_revision": 1, "language": "1.0"},
  "result": {
    "image": "img_89bc392c3a90213be3e450110f52eb22", "width": 512, "height": 512, "channel": "color",
    "annotate": "marks", "crop": {"left": 0.21, "top": 0.2, "right": 0.57, "bottom": 0.64},
    "format": "png", "bytes": 96412,
    "sha256": "2bcb43cbc8f6b7ef66331532881143fcbae60a879db3a8fb853f645bb24c2b3c",
    "path": "/home/user/.cache/vl/views/img_89bc392c3a90213be3e450110f52eb22-bowl-512-marks.png",
    "marks": [
      {"mark": 1, "object": "counter", "line": 6}, {"mark": 2, "object": "bowl", "line": 7},
      {"mark": 4, "object": "orange_2", "line": 9}, {"mark": 8, "object": "lemon", "line": 10}
    ]
  }
}
```

The same operation with `"as": "grid"` returns the pixels as text. The cells are illustrative; the operation does not exist yet.

<!-- schema: Response -->
```json
{
  "vlp": "0.1", "id": "14", "op": "view", "ok": true,
  "engine": {"name": "vl", "version": "0.2.0", "render_revision": 1, "language": "1.0"},
  "result": {
    "image": "img_89bc392c3a90213be3e450110f52eb22", "width": 16, "height": 9, "channel": "color",
    "annotate": "none", "crop": {"left": 0, "top": 0, "right": 1, "bottom": 1},
    "grid": {
      "columns": 16, "rows": 9, "encoding": "palette",
      "palette": [
        {"key": "a", "hex": "#beb5aa", "name": "light gray", "percent": 34},
        {"key": "b", "hex": "#8a552c", "name": "dark orange (brown)", "percent": 58},
        {"key": "c", "hex": "#afa099", "name": "medium gray", "percent": 3},
        {"key": "d", "hex": "#c9812e", "name": "medium orange", "percent": 3},
        {"key": "e", "hex": "#663c22", "name": "dark orange (brown)", "percent": 2}
      ],
      "cells": [
        "aaaaaaaaaaaaaaaa", "aaaaaaaaaaaaaaaa", "aaaaaaaaaaaaaaaa", "bbbbbcdcbbbbbbbb",
        "bbbbbcdcbbbbbbbb", "bbbbbbbbbebbbbbb", "bbbbbbbbbebbbbbb", "bbbbbbbbbbbbbbbb",
        "bbbbbbbbbbbbbbbb"
      ]
    }
  }
}
```

### 5.5 Failures of the call itself

<!-- schema: Response -->
```json
{
  "vlp": "0.1", "id": "11", "op": "render", "ok": false,
  "engine": {"name": "vl", "version": "0.2.0", "render_revision": 1, "language": "1.0"},
  "error": {
    "code": "invalid_params", "message": "'render' has no parameter 'sample'", "retryable": false,
    "param": "sample", "hint": "did you mean 'samples'?",
    "allowed": [
      "program", "quality", "width", "height", "samples", "seed", "denoise", "region_left", "region_top",
      "region_right", "region_bottom", "wait_ms", "max_seconds", "report", "diagnostics", "strict"
    ]
  }
}
```

<!-- schema: Response -->
```json
{
  "vlp": "0.1", "id": "12", "op": "render", "ok": false,
  "engine": {"name": "vl", "version": "0.2.0", "render_revision": 1, "language": "1.0"},
  "error": {
    "code": "limit_exceeded",
    "message": "this render needs 16384 x 16384 pixels; this server allows 8294400", "retryable": false,
    "limit": {"name": "pixels", "max": 8294400, "requested": 268435456},
    "hint": "ask for at most 3840 x 2160, or render a part of the picture with region_left ... region_bottom"
  }
}
```

### 5.6 Rules for parameters that a model fills in

A tool's parameter schema must be accepted by every provider's strictest mode. The documented restrictions differ [A1, A7, A10, A27], so the protocol stays inside what all of them accept:

- The root is an object. Members are strings, integers, numbers, booleans, strings from a fixed list, or arrays of those. No nested objects, no alternatives, no references, no recursion.
- Each tool has at most sixteen parameters, far below every published limit.
- Bounds and lengths are not written into the schema that a provider sees, because Anthropic's strict mode rejects numeric bounds [A7]. They are stated in each parameter's description, enforced by the server, and reported with `invalid_params` when violated.
- Each definition is generated in two variants. In the plain variant, optional parameters are simply not required. In the strict variant every parameter is required, optional ones also accept `null`, and additional members are forbidden, which is what OpenAI's and Anthropic's strict modes require [A1, A7]. The server treats `null` and absent alike.
- Tool names match `^[a-z][a-z0-9_]{0,63}$`. This satisfies the naming rules that Gemini and MCP document [A10, A12] and those reported for OpenAI and Anthropic [A27]. Every tool name starts with `vl_`.

## 6. The language specification

### 6.1 One source

The compiler gets a registry: declarative tables that describe every kind, property, relation, unit, function, preset, colour name, option word, limit and message code. The compiler reads its inputs through the registry. A property that is not registered cannot be read, and a registered property that is never read fails a test.

Much of this exists already as tables of names. What is missing is the rest of each entry. A property's type, default and range live today inside 41 reading calls of the form `self.num(props, ["fov"], 35.0, 1.0, 150.0)`. Those calls become lookups of registry entries, which moves the numbers to a place where a generator can see them without changing what the compiler does.

### 6.2 What is generated from it

| Artifact | Form | Use |
|---|---|---|
| `vl-language.json` | machine-readable, with its own JSON Schema | tools, editors, tests, adapters |
| `vl-language.md` | complete human-readable specification | people; models with large context windows |
| Reference cards at three sizes | Markdown with an enforced word budget | instructions for models; the smallest must fit beside a conversation in an 8,000-token window |
| Code registry | one entry per message code: default severity, meaning, the names and types of its `data` members, a program that triggers it, a corrected program | `reference` with a code as topic; documentation; tests |
| Grammar | EBNF in the specification, and the same grammar in the formats that constrained decoders accept | documentation; section 9.6 |
| Skill files | `SKILL.md` and its reference | shell agents |

All of them are embedded in the binary and returned by `reference` and `vl reference`. A model cannot read documentation for a different version than the one it is talking to.

### 6.3 Coverage of what you listed

| Must be documented | Where the truth is today | State today | In the registry |
|---|---|---|---|
| Shapes and geometry | `OBJECT_KINDS` (11 kinds with one-line summaries), `shape_props()` | kinds listed; types, defaults and ranges only inside the reading calls | per kind and property: type, unit, default, range, aliases, one example |
| Properties and parameters | eight property tables in `kinds.rs`, about 80 names | 26 names appear in neither reference file | every name; tested both ways as in 6.1 |
| Materials and textures | `MATERIAL_PROPS` (17 names), 33 presets and 9 aliases, 32 functions with parameter lists and doc strings | presets listed by name; 6 material properties undocumented | presets with the values they resolve to; functions with signatures |
| Units and measurements | unit table in the lexer, 13 suffixes | no example uses a unit | suffix, dimension, factor; the canonical unit of every property |
| Spatial relationships | `RELATIONS` (8), `PLACEMENT_PROPS` (12), vessel rules in `eval.rs` | described | what each relation computes, what it needs, which codes it can raise |
| Lighting | `LIGHT_PROPS` (21), `SKY_PROPS` (9), `ENVIRONMENT_PROPS` (5) | 2, 4 and 1 names undocumented | as for properties, plus the rule that decides between daylight and night |
| Camera | `CAMERA_PROPS` (10), automatic camera in `lower.rs` | 2 names undocumented | including what happens when no camera is written |
| Rendering options | `RENDER_PROPS` (15), `Film::default()` | 10 names undocumented | including the defaults and the quality presets |
| Error and warning codes | 62 codes written as text at 104 places | 20 documented, 42 not | an enumerated type: the compiler cannot emit an unregistered code |
| Syntax | hand-written lexer and parser | block comments, `;` and quoted names accepted but undocumented | grammar, tested against the parser |

### 6.4 Keeping it true

Four tests, extending today's `crates/vl-lang/tests/skill_docs.rs`:

1. **Regeneration.** The generated files are committed. The test regenerates them and fails on any difference.
2. **Properties in both directions.** For every registered property, a program that sets it to a value inside its range must compile without an "unknown property" diagnostic, and a value outside must produce the documented code. For every block kind, a property name that is not registered must be reported.
3. **Every code is reachable.** Each of the 62 codes has a test that triggers it: a program wherever a program can cause it, a unit test otherwise. The programs are the examples in the code registry.
4. **Grammar against parser.** The grammar cannot be derived from a hand-written parser, so a corpus of accepted and rejected programs is checked against both, and programs generated from the grammar must be accepted by the parser.

### 6.5 Tolerance is documented, not hidden

The compiler repairs near misses such as `radus` and reports them as warnings W340 to W351. The specification defines the canonical language and lists each repair separately with its code. A `strict` parameter turns the repairs into errors, so an evaluation can count "valid without help" separately from "valid".

### 6.6 Small language changes this design asks for

Each one needs your approval, because each changes the language:

- A leading byte-order mark is ignored.
- A program may state the language version it was written for, so that a future engine can tell an old program from a wrong one.
- Names have a maximum length.
- Open question: names are limited to ASCII letters, digits and underscore today. A model working in another script will try `球`. Either that becomes legal or the message says clearly what to do instead.

## 7. Versioning strategy

Four things change at different speeds and are versioned separately.

| What | Form | Changes when | Breaking means |
|---|---|---|---|
| Protocol | `major.minor` | operations, parameters, result members or error codes change | anything removed or renamed; a meaning or default changed; a new required parameter |
| Language | `major.minor` | syntax, kinds, properties, functions, presets, units or codes change | a valid program becomes invalid or means something else |
| Render revision | a whole number | the pixels of an unchanged program may differ | every step, by definition |
| Engine | semantic version | every release | follows from the three above |

Rules:

- **Minor versions only add.** New operations, new optional parameters, new result members, new codes, new feature names.
- **Features are discovered, not inferred.** A client asks `capabilities` whether `view.marks` exists. It does not compare version numbers.
- **Every request names its version.** There is no handshake and no session. A server that supports the stated major version answers in it. Otherwise it returns `unsupported_version` with the list it supports. MCP moved to the same per-request model in its July 2026 revision [A13].
- **Before 1.0, anything may change.** The first release is 0.1. Version 1.0 is declared only after the tests of section 13 have run against real models from several providers, because that is when mistakes in the tool descriptions and schemas will show.
- **Codes are permanent.** A message code is never reused and never changes meaning. Message text may change and must not be parsed.
- **Deprecation before removal.** A deprecated feature is marked in the registry, produces a warning, and is removed only with the next major language version.
- **The render revision makes reproducibility explicit.** It is part of every picture identifier and of every response. Reference hashes used by tests are stored per revision. A change to the renderer that alters pixels raises the revision and updates the hashes in the same commit, where a reviewer can see it.
- **Generated files state their origin.** Each set of tool definitions comes with a manifest giving the protocol version and a hash of each file.

## 8. Error-handling specification

### 8.1 Three kinds of message that must not be confused

| Kind | Example | Where it appears | Who acts on it |
|---|---|---|---|
| Failure of the call | unknown parameter, picture expired, limit exceeded | `ok: false` with an `error` | the calling software, or the model when its arguments caused it |
| Diagnostic about the program | E201 unknown name, W320 object floats | `diagnostics` inside a successful result | the model |
| Finding about the picture | V-codes: object hidden, cut off by the frame, overexposed | `report.findings` inside a successful result | the model |

A program with mistakes is not a failed call. `check` succeeded: it found the mistakes. `render` returns status `rejected` with the same diagnostics. This keeps one rule for every consumer: `ok: false` means the software around the model has something to handle.

### 8.2 Failures of the call

| Code | Meaning | Who can fix it | HTTP status | Exit status | Retry |
|---|---|---|---|---|---|
| `invalid_request` | not JSON, or not a request | application | 400 | 2 | no |
| `unsupported_version` | lists the supported versions | application | 400 | 2 | no |
| `unknown_operation` | | application | 404 | 2 | no |
| `invalid_params` | a parameter is unknown, missing, of the wrong type or out of range; names it and lists the accepted ones | model | 400 | 2 | after correction |
| `too_large` | program or request over the size limit | model | 413 | 3 | after correction |
| `limit_exceeded` | more pixels, samples, time or memory than allowed; names the limit, the maximum and what was asked | model | 422 | 3 | after correction |
| `not_found` | unknown or expired picture or job; unknown reference topic, with suggestions | model | 404 | 3 | after rendering again |
| `busy` | too many renders running or waiting | application | 429 | 3 | yes, after `retry_after_ms` |
| `cancelled` | the job was cancelled | | 409 | 3 | yes |
| `deadline_exceeded` | the time budget ran out | model or operator | 504 | 3 | with a smaller request |
| `unauthorized` | token missing or wrong | operator | 401 | 2 | no |
| `internal` | a defect in the engine; carries an incident number, never a stack trace or a path | maintainer | 500 | 4 | no |

Exit statuses 0, 1 and 2 keep today's meaning: success, program has errors, wrong usage or input and output failure. Statuses 3 and 4 are new. Today a panic exits with 101 and a stack overflow with 134.

Every error has a one-sentence message and, where one exists, a hint that says what to do next, written to be read by a model.

### 8.3 What a model sees

Adapters apply one rule. A failure that the model can fix is returned to the model as the result of its tool call, marked as an error where the provider has such a mark: `is_error` in Anthropic's format [A5], `isError` in MCP [A12]. MCP's specification draws the same line: malformed requests are protocol errors, while input validation failures are tool results the model can correct [A12]. A failure that the model cannot fix (`unauthorized`, `internal`, `unsupported_version`) is raised to the application instead.

### 8.4 Diagnostics

The full shape is in section 5.2. The rules:

- The complete list is always available. The condensed form that today's compiler applies unconditionally (errors only while errors exist, at most three warnings per kind) becomes the `focused` setting. The totals in `counts` are complete in both settings.
- `source_line` is limited to 240 characters and has control characters removed (section 11.6).
- A panic inside an operation is caught at the protocol boundary and reported as `internal`.

## 9. Model adapter architecture

### 9.1 What an adapter may and may not do

An adapter may:

- choose which operations and which optional parameters a model sees (the tool profile);
- give fixed values to parameters it hides, for example `wait_ms`, declared in a manifest;
- decode the provider's representation of a call and encode the result in the provider's representation;
- decide where a picture goes in the conversation, from the capability profile;
- choose the size of the reference card and the diagnostics setting from the model's context size.

An adapter may not:

- alter the program text in any way;
- drop, reorder or reword a diagnostic;
- compute anything about the scene;
- contain the name of a model.

The enforcing test: every recorded request is sent through each adapter and directly to the engine, and the diagnostics and the pixel hash must be identical.

One kind of input repair does belong to the protocol layer, where it happens once and identically for everyone: removing a byte-order mark, and removing a Markdown code fence when the whole text is exactly one fenced block. Each repair is reported in `program.normalized`. Anything less clear-cut is an error with a hint.

### 9.2 Five ways a model can take part

The division is by ability, not by vendor or size.

| The model can | How it takes part | Generated for it |
|---|---|---|
| call tools | tool definitions in its provider's format | definitions in every format and profile |
| produce JSON that follows a schema | one fixed reply object: an action (`render` or `done`) and a program | the reply schema |
| produce text that follows a grammar | the program itself, constrained so that it cannot be syntactically wrong | grammar files |
| only chat | one fenced `vl` block per reply; the feedback returns as text | instructions; a precisely specified extraction rule |
| run shell commands | the command line and the agent skill | `SKILL.md` and its reference |

The "only chat" row is what the experiment script does today. It is specified so that every plain-chat integration extracts the program the same way.

### 9.3 Provider formats

From documentation read on 2026-11-15. None of it is tested against a live API. In file names and parameters a provider format is called a dialect.

| Format | Tool definition | Arguments arrive as | Result is returned as | Picture inside a tool result | Source |
|---|---|---|---|---|---|
| OpenAI Chat Completions, and the many servers that copy it | `{"type":"function","function":{name, description, parameters, strict}}` | a JSON string | a message with role `tool`; text only | **no**: "For tool messages, only type text is supported" | A1, A2 |
| OpenAI Responses | `{"type":"function", name, description, parameters, strict}`; also free-text "custom" tools | a JSON string in a `function_call` item | a `function_call_output` item; a string, or an array of text, image and file parts | yes | A1 |
| Anthropic Messages | `{name, description, input_schema}` | an object in a `tool_use` block | a `tool_result` block; a string, or an array of text, image and document blocks; `is_error` | yes | A5 |
| Gemini Interactions | `{"type":"function", name, description, parameters}` | an object in a `function_call` step | a `function_result` with an array of text and image parts | yes | A8 |
| Gemini generateContent | `functionDeclarations: [{name, description, parameters}]` | an object in `functionCall` | `functionResponse` with nested `parts` carrying `inlineData` | yes, for Gemini 3 series models | A9 |
| Ollama, native | as OpenAI chat | an object | a message with role `tool` and `tool_name`; text | no; pictures go in the `images` list of a user message | A15 |
| MCP 2026-07-28 | `{name, title, description, inputSchema, outputSchema}` | an object | `content` parts (text, image, resource link) and `structuredContent`; `isError` | yes; whether the host shows it to the model is the host's decision | A12 |

Four observations follow.

- **Two formats reach most models.** DeepSeek, Qwen through DashScope's compatible mode, Ollama and llama.cpp all accept the OpenAI chat format [A16, A18, A19, A26]. llama.cpp and Ollama also accept Anthropic's [A17, A19]. A provider that adopts an existing format needs no new adapter, only a capability profile.
- **"OpenAI-compatible" now means two formats.** OpenAI's guide says its newest models require the Responses API for tool calling [A1], while third-party servers implement Chat Completions. Both are generated.
- **Gemini has two APIs.** Google's documentation now labels generateContent "Legacy" and presents the Interactions API as current [A8, A9]. Both are generated.
- **Formats change within months.** MCP's July 2026 revision removed the handshake and the sessions of the previous one [A13]. This is why adapters are kept thin, generated and tested against recorded messages rather than written by hand around an SDK.

Appendix C shows one operation in each format.

### 9.4 Capability profile

Everything an adapter needs to know about a model is in a small profile that the integrator supplies:

```json
{
  "dialect": "openai-chat",
  "acts_by": "tools",
  "strict_schemas": false,
  "reads_images": false,
  "image_in_tool_result": false,
  "image_max_side": 1024,
  "image_max_bytes": 3000000,
  "images_per_turn": 2,
  "context_tokens": 8192,
  "tool_profile": "minimal"
}
```

The defaults are the most conservative: text only, the smallest tool profile, the short card. Example profiles may be shipped as dated data files that are clearly labelled as examples. No code consults a model name. There is no reliable way to ask an API whether a model reads images, so the profile is stated, not detected.

### 9.5 Tool profiles

| Profile | Tools | Parameters shown | Size of the definitions |
|---|---|---|---|
| minimal | `vl_render` (program only), `vl_reference` (topic only) | 2 | about 1,100 to 1,200 characters |
| standard | adds `vl_check`, `vl_view`, `vl_job` | 15 | about 3,700 to 4,300 characters |
| full | the same five tools with every parameter | 40 | about 6,300 to 7,500 characters |

The sizes are of draft definitions that I derived mechanically from the draft schema. The lower figure is the plain variant in Anthropic's envelope, the higher the strict variant in OpenAI's. At roughly four characters per token these are about 300, 1,000 and 1,800 tokens. The descriptions are prompts in their own right and will change once real models have used them. A description is assembled from sentences, each tied to the parameter or tool it mentions, so a profile never mentions something it hides.

In the minimal profile the adapter fixes the quality to `preview` and declares that in its manifest, so the model only ever renders previews. When the model declares the picture finished, the driver, not the model, requests the final quality. A small model then has one decision to make per turn.

### 9.6 Local and small models

- **Format.** Local servers accept the OpenAI chat format, so they need no adapter of their own.
- **Constrained output.** llama.cpp accepts a grammar or a JSON Schema with a request and restricts the model's output to it [A19]. Ollama accepts a JSON Schema [A16]. OpenAI's custom tools accept a grammar as well [A1]. Generating a grammar file from the language's grammar means a small model cannot produce a syntactically invalid program at all. Whether that also improves the scenes is a question for the evaluation, not a claim.
- **Small context.** The card has a word budget that a test enforces. The minimal tool profile costs about 300 tokens. The `focused` diagnostics setting exists for these models.
- **Offline.** The engine makes no network connection, and the reference is inside the binary.

### 9.7 The MCP adapter

It is a separate crate behind its own subcommand, `vl mcp`. Nothing else depends on it, and removing it changes nothing else.

- It speaks JSON-RPC on standard input and output. That needs only the JSON code that already exists, so no SDK is involved.
- It implements the current revision, 2026-07-28, and also answers the older `initialize` handshake, because hosts built for earlier revisions use it. The revision's own documentation describes how clients tell the two eras apart [A14].
- Long renders use the protocol's own `vl_job` tool, which works in every host. The Tasks extension is used in addition when a client announces it.
- Reference sections and pictures are also offered as MCP resources.

### 9.8 Reference driver

A driver is the loop that sends a conversation to a model, executes the tool calls and returns the results. It is not part of the engine. A small one is provided in Python, using only the standard library, for three reasons: as a worked example for each format, as the harness for the tests in section 13, and because the experiment script already contains most of it (the loop, two backends, the feedback wording).

## 10. Image delivery and inspection

### 10.1 A stored picture and its views

`render` stores one picture, the PNG at the rendered size. Everything a model looks at is a view derived from it by `view`:

| Parameter | Effect | Exists today? |
|---|---|---|
| `max_side` | scales down; never up | no; `vl-image` has no scaling function |
| `crop_object`, `crop_margin` | the part of the picture around one object, found from the object-per-pixel buffer | cropping exists; the lookup does not |
| `crop_left` … `crop_bottom` | an explicit rectangle | cropping exists |
| `annotate: marks` | a number drawn on every visible object; the result lists number, object name and source line | no |
| `channel` | `objects` paints each object in one flat colour; `depth`, `normals`, `albedo` | the buffers exist; writing them out does not |
| `as: grid` | the pixels as rows of letters with a colour legend, or as hex values | no |

Marks deserve a note. Overlaying numbered marks on image regions was shown to improve how well a vision model refers to regions [A21]. That work had to estimate the regions with a segmentation model. Here the renderer knows exactly which object each pixel belongs to, so the marks cost nothing and cannot be wrong. Whether they help models correct scenes is an assumption that the evaluation will test.

### 10.2 How a picture reaches each kind of model

| Path | Mechanism |
|---|---|
| Format allows pictures in tool results (OpenAI Responses, Anthropic, Gemini, MCP) | the view is attached to the tool result as an image part |
| Format allows only text in tool results (OpenAI chat and the servers that copy it, Ollama) | the text goes into the tool result; the view follows in a user message as an image part. Ollama's compatible endpoint accepts base64 data only [A18] |
| Model does not read images | no picture is sent. The model gets the report, and on request the letter grid and colours at points. The file is still stored and retrievable. |
| Shell agent | the result gives a file path; the agent opens it with its own means |
| HTTP client | the result gives a URL; `GET` returns the bytes |

In every case the complete file remains retrievable through `view`, a path or a URL. This is what satisfies "retrieve the actual rendered image" for every model, including those that receive only the report in conversation.

### 10.3 Sizes and what they cost

| View | PNG (measured) | As base64 | Anthropic image tokens | OpenAI 32-pixel patches |
|---|---|---|---|---|
| 512 × 288 | 142 KB | 0.19 MB | 19 × 11 = 209 | 16 × 9 = 144 |
| 1024 × 576 | 437 KB | 0.58 MB | 37 × 21 = 777 | 32 × 18 = 576 |
| 1568 × 882 | 914 KB | 1.22 MB | 56 × 32 = 1,792 | 49 × 28 = 1,372 |
| 3840 × 2160 | 6.7 MB | 8.95 MB | scaled down by the provider | depends on model and detail level |

The token columns apply the formulas in the providers' documentation [A3, A6]. Documented limits on the day of reading: Anthropic accepts up to 8000 × 8000 pixels and 10 MB per image as base64 on its own API, 5 MB on Amazon Bedrock, and scales images down to 1568 pixels on the long side, or 2576 on its newest models [A6]. Gemini limits a request with inline data to 20 MB in total [A11]. OpenAI accepts PNG, JPEG, WEBP and GIF, and resizes within model-specific budgets [A3]. These numbers change and belong in capability profiles, never in the engine.

The conclusions for the design:

- The default view is at most 1024 pixels on the long side and stays far inside every documented limit.
- The full 4K file is never sent to a model. At 8.95 MB as base64 it exceeds the 5 MB limit of one platform and is close to the 10 MB limit of another, and the provider would scale it down anyway. A model examines detail through crops.
- PNG is accepted everywhere and is the only required format. JPEG would make views smaller and is optional.

### 10.4 The report is measured from the delivered picture

Today the colours and positions come from a separate 256-pixel render made for inspection. In the design, everything about light and colour (each object's apparent colour, brightness, clipped highlights and shadows, remaining noise) is measured on the pixels of the delivered picture, using the object-per-pixel buffer of the same render. Facts about geometry that do not depend on resolution (which object hides which, how much of an object is visible) may still come from the fixed-size pass.

As a side effect, a preview render no longer needs the separate inspection render. Today `vl render` at 512 pixels spends 3.9 s rendering and about 1.7 s on that second pass.

### 10.5 Looking closely without rendering everything

Checking a detail of a 4K picture should not take 17 minutes. `render` accepts a region as four fractions. The integrator already supports rendering a rectangle; the command line does not expose it. Rendering a tenth of the width and height at the final resolution costs about a hundredth of the rendering time, plus the fixed cost of building the scene and measuring the exposure.

### 10.6 Progressive refinement (optional, later)

Because every sample has its own random stream keyed by seed, pixel and sample number, the first 16 samples of a 256-sample render are exactly the 16-sample render at the same size, before denoising. A long job could therefore publish intermediate pictures, and a cancelled job could return what it has. This needs the integrator to keep running sums per pixel. I have read the code but not tested this, and it is not required for the first version.

## 11. Security and execution isolation

### 11.1 What is being protected against what

The engine runs programs written by a model. That model may be mistaken, may be steered by text it read elsewhere, or may be replaced by a person with bad intent when the engine is offered as a service.

| What could be harmed | How | Present state |
|---|---|---|
| Files, secrets and network of the host | a program that reads, writes or connects | Not possible. The language has no construct for it and the engine crates contain no file, network, process or environment access (section 3.1). |
| Availability | a program or request that takes unbounded time or memory, or crashes the process | Possible today (section 3.3) |
| The host's file system through outputs | an output path chosen by the model | `-o PATH` writes wherever the caller says. Acceptable for a person at a shell; not acceptable as a tool parameter. |
| The model that reads the results | text placed in a program that comes back in a diagnostic and is taken as an instruction | Possible in principle: `source_line` echoes program text, including comments |
| Other users of a shared server | guessing identifiers; reading someone else's pictures | No server exists yet |

### 11.2 Rules that follow

1. **The language never gains ambient access.** When textures, environment maps and meshes arrive, a program refers to them by a name declared in the request. It never refers to a path or a URL. This removes path traversal and server-side request forgery by construction.
2. **Tools do not take output paths.** Pictures go to the store, and the result says where they are. The command-line option `-o` remains for direct use by a person or a shell agent, which already has those rights.
3. **Every limit is enforced before work is done, and is listed in `capabilities`.**
4. **Results are data.** An adapter never places tool output in a system or instruction position.

### 11.3 Isolation

**A worker process per render.** A server, and the detached mode of the command line, start a separate `vl` process for each render. They pass the request on its standard input and read progress and the result from its standard output. This gives four things the engine cannot give itself:

- a hard stop, by ending the process, for cancellation and time limits;
- containment of crashes, including the stack overflows that cannot be caught;
- a place where the operating system can limit memory;
- no shared memory between jobs.

Until the defects of section 3.3 are fixed and a fuzzing campaign has run clean, a server runs `check` in a worker as well, with a time limit of a few seconds.

**Inside one process**, for the library, WebAssembly and the ordinary command line:

- a cancellation flag and a deadline, checked between bands of rows in the integrator and in the denoiser and the PNG encoder;
- every recursion bounded, and every pass that grows faster than linearly either counted against the work budget or rewritten;
- panics caught at the protocol boundary and reported as `internal`.

**WebAssembly** is a second isolation boundary worth having later: a module has a fixed memory ceiling and no access to anything it is not given. `par.rs` already has a single-threaded path for it. It has never been built or tested.

**For deployments**, the documentation gives a container recipe: read-only file system, no outgoing network, an unprivileged user, and memory, processor and process-count limits. The engine has no dependencies and cannot set operating-system limits itself without unsafe code, so it relies on the environment for these.

### 11.4 Limits

The proposed values are starting points to be tuned.

| Limit | Today | Proposed, local use | Proposed, server |
|---|---|---|---|
| Program size | 4 MiB | 4 MiB | 256 KiB |
| Compile work | 2,000,000 evaluation steps; several passes uncounted | every pass counted | the same |
| Compile time | none | 10 s | 2 s |
| Objects | 500,000 | 500,000 once the pairwise pass is fixed | 20,000 |
| Nesting | 64 for blocks and brackets; prefix operators and long chains unbounded | 64 for everything | the same |
| Name length | none | 64 characters | the same |
| Pixels | 16384 × 16384 | 3840 × 2160; more with an explicit flag | 3840 × 2160 |
| Samples | 65,536 | 4,096 | 1,024 |
| Render time | none | none; cancel with `vl job` | 1,800 s |
| Memory | none | estimate compared with a configured ceiling before starting | the same |
| Renders at once | | 1 | 1 running, 8 waiting |
| Stored pictures | | 2 GiB, 24 hours | configured |
| Diagnostics returned | unbounded | 200, the rest counted | the same |

The memory estimate uses the measured 105 bytes per pixel with the denoiser (874 MB at 3840 × 2160) and 64 without, plus the scene.

### 11.5 The HTTP server

- It listens on the loopback address unless told otherwise. On any other address it refuses to start without a token.
- It checks the `Origin` and `Host` headers, which stops a web page in the user's browser from reaching a local server. MCP requires the same of its HTTP servers for the same reason [A14].
- It limits body size, header size and time per request, closes every connection after one request, and accepts only `GET`, `POST` and `DELETE`.
- Identifiers in paths are matched against a fixed pattern before any file is opened. The draft schema already rejects `../../etc/passwd` as a picture identifier.
- It does not do TLS. A hand-written TLS implementation would be a liability. Remote use goes through an established reverse proxy, which also provides HTTP/2 and rate limiting.
- On a shared server every stored picture belongs to the token that created it. An identifier is a name, not a key.
- Logs record operation, size, duration and outcome. They do not record program text unless that is switched on.

Writing a small HTTP/1.1 server with only the standard library keeps the project's no-dependency rule. It is also the most security-sensitive new code in the design. The alternative is one small, widely used HTTP crate in the server crate only. Section 18 asks you to choose.

### 11.6 What goes back to the model

- `source_line` is cut to 240 characters and stripped of control characters. The server may omit it. Names are limited in length.
- The PNG carries five text entries today: the software name and version, a hash of the source, the seed, the sample count and the exposure. It carries no path, no host name, no user name and no time. That stays the rule.

### 11.7 Supply chain

- The engine and protocol crates keep zero third-party dependencies. This is true today.
- Release binaries are built by a workflow from a tagged commit and published with checksums and a signed build attestation that anyone can verify with `gh attestation verify` [A25].
- Provider SDKs are never a dependency of anything shipped. The Python driver uses the standard library's HTTP client.

### 11.8 Privacy

The engine makes no network connection, and a test keeps it that way. Model providers see only what the integrator's own driver sends them. The existing experiment script sends prompts and programs to a third-party endpoint. It is a research tool, it stays outside the product, and whether to keep using that endpoint is still your open decision.

## 12. Installation and distribution

| Channel | What the user does | Status |
|---|---|---|
| Source | `cargo build --release` | works today; the only way that exists |
| Release archives on GitHub | download and unpack one file per platform, with checksums and attestation | to build. Only Linux x86-64 has ever been compiled. |
| Python package | `pip install NAME` or `uvx NAME` puts the binary on the path; includes a pure-Python client | to build. maturin can package a Rust binary as a wheel [A23]; it is a build tool, not a dependency of the result. |
| npm package | `npx NAME` | to build, with the usual per-platform optional packages [A24]. `npx` is the customary way MCP hosts start a server. |
| Container image | `docker run` | to build; for the HTTP server |
| WebAssembly module | loaded by a browser or a WASI runtime | untested. Rust lists the WASI targets as supported with the standard library [A22]. |
| Agent skill | copy the folder `skills/vl-scene/` into the agent's skills directory | exists. `SKILL.md` is now a cross-vendor format with a published specification [A20], and today's file already satisfies its naming and length rules. Its content is to be regenerated and its wrong claim corrected. |

Target platforms for the archives: Linux x86-64 and ARM64 (statically linked), macOS ARM64 and x86-64, Windows x86-64. The binary is 1.8 MB and needs nothing at run time.

The command `vl version --json` reports the engine version, the protocol and language versions, the render revision, the commit and the build target, so a bug report or a test log identifies exactly what ran.

Four facts stand in the way of distribution, and each is yours to decide:

- The repository is private. Release archives in a private repository are not publicly downloadable.
- No licence file exists (section 3.4).
- The name `vl` is taken on both PyPI and npm. I checked today that `vl-scene`, `vlscene`, `vl-render` and `vl-engine` are free on both. I could not check crates.io from the sandbox.
- As far as I know, macOS and Windows runners are billed at a higher rate than Linux in private repositories; I have not checked the current rates. The release workflow should run on tags only.

## 13. Testing strategy across providers

| Layer | A model needed? | Cost | When | What it proves |
|---|---|---|---|---|
| 1. Engine tests (113 today) | no | none | every push | the engine |
| 2. Protocol conformance | no | none | every push | every binding returns the same result and the same pixel hash for recorded requests; all messages satisfy the schemas |
| 3. Specification tests (section 6.4) | no | none | every push | the reference equals the implementation |
| 4. Format conformance | no | none | every push | the generated definitions satisfy each provider's documented rules; adapters read and write each provider's documented messages |
| 5. Robustness | no | none | short run on every push, long run nightly | no crash and no stall on hostile input |
| 6. Live checks | yes | small; needs API keys | by hand, before a release | a real API accepts our definitions and our results; a picture really reaches the model |
| 7. Capability evaluation | yes | larger | by hand | how well each class of model produces correct scenes |

**Layer 2** also runs on Linux, macOS and Windows and on both processor families and compares pixel hashes. This is the cross-platform determinism check that has been open since the first status report. If the hashes differ, picture identifiers and reference hashes become per-platform and the documentation says so.

**Layer 4** has three parts, none of which needs a key:

- Checkers that encode each provider's documented rules: name pattern, allowed schema keywords, what strict mode requires. The generated definitions must pass all of them.
- Recorded provider messages, taken from the documentation at first and from real sessions later. Each adapter must decode the recorded calls and produce exactly the recorded results, including where the picture is placed.
- A local stand-in server that speaks each provider's format and plays scripted conversations. It drives whole loops through driver, adapter and engine, with no cost and no network.

Passing layer 4 shows agreement with the documentation as I read it. It does not show that a live API accepts the messages. Only layer 6 shows that.

**Layer 6** asserts mechanics, not quality. For each provider:

1. The API accepts the tool definitions.
2. For a trivial request the model produces a well-formed call.
3. The API accepts the tool result.
4. The picture arrives. The harness renders a scene the model has never seen, with colours and counts chosen at random, returns it through the tool-result path and asks what colour the object marked 3 has. The answer is known, so the check is objective.

**Layer 7** is the actual research question, and it needs the two things the first two experiments lacked.

- *Machine-checkable criteria.* Each task states what must hold, as predicates over the report: an object of a kind exists, its colour is near a named colour, it rests on another object, a stated share of it is visible, the picture has no findings. Scoring no longer rests on my judgement. Predicates cannot judge realism, so a human review remains for any claim about how the pictures look.
- *Enough repetitions.* The earlier experiments had 9 and 12 runs in total. Each cell needs repeated runs, and results are reported with their uncertainty.

The models are chosen by class, with at least one per cell and two providers where a cell is hosted:

| | Calls tools and reads images | Calls tools, text only | Constrained output only | Chat only |
|---|---|---|---|---|
| Large hosted | GPT, Gemini, Claude | DeepSeek, Qwen | | any, as a baseline |
| Mid-size open weights | a vision-capable Qwen or Llama variant | Llama, Qwen | the same with grammar | the same |
| Small local, 1.5 to 3 billion parameters | | where the model's template supports tools | the main route for this class | today's baseline |

The placements are examples; a family can sit in several cells.

What this needs from you: API keys for the hosted providers, stored as repository secrets or used on your own machine; or one key for a routing service, which is acceptable for layer 7 but not for layer 6, since layer 6 exists to test each provider's own format. Without keys I can run layers 1 to 5 completely and layers 6 and 7 only for local models on GitHub's runners.

## 14. Existing components that can be reused

Nothing is deleted. Every existing command and option keeps working.

| Component | Verdict | What changes |
|---|---|---|
| `vl-core`: math, random numbers, colour | reuse unchanged | add SHA-256 for identifiers and checksums |
| `vl-core`: JSON parser and writer | reuse; becomes the protocol's parser | review against hostile input; the depth limit exists |
| `vl-core`: diagnostics and suggestions | reuse | codes become an enumerated registry; `not_found` reuses the suggestion code |
| `vl-core`: thread pool | reuse | cancellation flag |
| `vl-image`: PNG, deflate, image, crop | reuse | add scaling down, drawing of marks with a small built-in digit font, the letter grid; JPEG optional |
| `vl-ir` | reuse unchanged | |
| `vl-lang`: lexer, parser, evaluator, lowering | reuse | fix the defects of 3.3; return all diagnostics and make trimming a separate function |
| `vl-lang`: property, relation and kind tables | reuse as the seed of the registry | add type, unit, default and range to each entry (41 reading calls) |
| `vl-lang`: function table and presets | reuse; already carry parameter lists and doc strings | |
| `vl-render`: integrator | reuse | cancellation; expose the region; optional per-pixel sums |
| `vl-render`: denoiser, post-processing | reuse unchanged | cancellation checks |
| `vl-render`: inspection | reuse the analysis | light and colour measured from the delivered picture |
| `vl-cli` (242 lines) | keep every command and option | becomes one binding; its JSON building moves into the protocol crate; today's `--json` output stays available |
| 113 tests | reuse | the 33 model-written programs become a corpus for diagnostics tests |
| `examples/` | reuse as conformance scenes | |
| `skills/vl-scene/` | reuse the structure | content generated; wrong claim corrected |
| `tools/ci/llm_experiment.py` | reuse the loop, both backends and the feedback wording | split into driver and adapters; add tool calling |
| Workflows | reuse the matrix and the way results are published | add a release build |
| `docs/` | keep | |

New components: the protocol crate (catalogue, schemas, dispatch, picture store, job manager, limits); the registry and its generators; the view functions; the JSON-lines and HTTP servers; the tool-definition generator; the Python adapters and driver; the optional MCP crate; the stand-in provider server and recorded messages; the conformance requests; the release workflow and packaging.

## 15. The engine behind the interface

You said the goal is an image creation engine capable of photorealistic rendering, not a universal wrapper around the current renderer. Three things follow for this design.

**The interface does not raise the ceiling.** The pictures today are clean computer graphics of simple solids and vessels. A more capable model reaches the limit of the scene vocabulary sooner, not later. Nothing in this document changes that. What it does is make sure that when the engine grows, every model gets the new ability on the same day.

**That is why the protocol knows nothing about the vocabulary.** A program is opaque text to it. Kinds, properties and materials reach a model through `reference`, which is generated from the registry. Adding meshes, constructive solid geometry, lathed and extruded shapes, image textures or volumes means adding registry entries and engine code. No schema, no tool definition and no adapter changes.

**The places where growth needs room are reserved now:**

- *Assets.* Textures, environment maps, meshes and reference photographs enter through a reserved `asset` operation and are named in programs, never addressed by path (section 11.2).
- *Image editing.* A reserved `edit` operation works on stored pictures and returns stored pictures. The whole feedback loop then applies to edits unchanged: `view`, `measure` with comparison, marks.
- *Extra channels.* Depth, normals, albedo and the object map are already computed. Exposing them serves inspection now and compositing later.
- *Long work.* Jobs, regions and, later, progressive refinement.

**The interface is also the measuring instrument for the engine.** The evaluation of section 13 shows where models fail. The two experiments already worked this way: what the models got wrong decided what was changed in the language and the compiler afterwards (`docs/experiments/`). Decisions about what to build next in the engine should come from that data.

I recommend that the engine roadmap gets its own design document of this kind before any of it is built, and that after a first thin slice of the protocol, engine work and interface work proceed side by side (section 16).

## 16. Suggested order of work

Nothing here has started. Each step leaves every existing command working, and each ends with tests that pass.

| Step | Content | Size | Depends on |
|---|---|---|---|
| 0. Hardening | Fix the two stack overflows and the two slow passes; ignore a byte-order mark; add the cancellation flag and deadline; pixel and sample ceilings; correct the skill's wrong sentence; a first fuzzing run | small | your approval only |
| 1. Protocol core | Protocol crate with `check`, `render`, `job`, `capabilities`; picture store; job manager; `vl call` and `vl serve --stdio`; full diagnostics with the trimming made a setting; conformance requests | large | 0 |
| 2. Registry and specification | The registry; generated JSON, Markdown and cards; code registry; the four tests; `reference` | large | 1 for `reference` only |
| 3. Views | Scaling, crops by object, marks, channels, grid; report from the delivered picture; regions; quality presets | medium | 1 |
| 4. Adapters | Definition generator for all formats and profiles; Python adapters and driver; stand-in server; recorded messages; regenerated skill | medium | 1 to 3 |
| 5. HTTP, then MCP | HTTP server with OpenAPI description; the optional MCP crate | medium | 1 |
| 6. Packaging | Release workflow, archives, Python and npm packages, container; the cross-platform determinism check | medium | your decisions in section 18 |
| 7. Evaluation | Live checks and the capability evaluation | medium, plus API costs | 4, and keys |

Small means a few hundred lines of new code, medium up to about a thousand, large more than that. I have not attached durations. I would rather estimate each step when it is approved.

After step 1 the protocol exists end to end through one binding. That is the point where engine work can resume in parallel without being disturbed by the rest.

## 17. Risks, and what this design does not solve

- **A better interface may not make small models succeed.** In the two experiments the only picture I judged correct came from the hosted model; none came from a 1.5 or 3 billion parameter model. Tool calling, constrained output, fuller diagnostics and visual feedback are reasonable hypotheses. They are untested.
- **Visual feedback is unproven here.** No model has yet been shown a picture from this engine.
- **Everything about provider formats is from documentation.** Until layer 6 runs, an adapter may be wrong in a way the documentation did not reveal. Some figures came from secondary sources and are marked in Appendix A.
- **Providers change.** MCP removed sessions in July 2026, Google now labels one of its two APIs as legacy, and OpenAI's newest models require the newer of its two. Maintenance of adapters is a permanent cost. The design keeps it small; it cannot remove it.
- **Cross-platform determinism is unverified.** Picture identifiers, the result cache and the reference hashes all depend on it. If it fails, they become per-platform.
- **A hand-written HTTP server is a security risk** that I would be introducing. The JSON-lines binding carries none of it and comes first.
- **The defects in section 3.3 are a sample.** A few minutes of probing found four. A server must not be exposed before step 0 and a fuzzing campaign.
- **The descriptions are prompts.** Tool and parameter descriptions affect model behaviour as much as the schemas do. The drafts in Appendix C are a first guess.
- **Scope.** This is a large amount of interface work for an engine whose pictures are not yet photorealistic. Section 16 is ordered so that work can stop after any step and leave something whole.
- **Left out on purpose:** a hosted service with accounts and billing, TLS, a graphical interface, animation, and the engine roadmap itself.

## 18. Decisions needed from you

1. **Architecture.** Do you approve the overall shape: a catalogue of operations, no hidden state, explicit handles, jobs for long renders, flat parameters?
2. **Operations.** Are the seven operations and their names right? Is anything missing from your loop?
3. **Versions.** Start at 0.1 and declare 1.0 only after live tests with several providers?
4. **Dependencies.** The engine and protocol crates stay at zero. For the HTTP server: hand-written with the standard library only, or one small established HTTP crate in the server crate alone?
5. **Language changes** (section 6.6): ignore a byte-order mark; an optional version statement; a maximum name length; and whether names may contain non-ASCII letters.
6. **Step 0.** May I fix the defects of section 3.3 first and separately? They exist regardless of this design.
7. **MCP.** Target the 2026-07-28 revision with a fallback for older hosts, as the last binding rather than the first?
8. **Distribution.** Will the project be public? Which licence files should I add? Which package name?
9. **Testing with real models.** Which provider keys can you supply, and with what budget? Is a routing service acceptable for the capability evaluation? Is the third-party endpoint used so far still acceptable? That question remains open from the last report.
10. **Order.** Do you want the engine roadmap designed in the same way before implementation starts, or the first steps of this design first?
11. **Acceptance criteria.** Still open: your own wording of what "done" means, now that the earlier proposal is set aside.

## Appendix A. Sources

All read on 2026-11-15. "Primary" means the provider's or standard's own documentation. "Secondary" means a third party describing it; those claims should be rechecked before code depends on them.

| Key | Source | Kind | Used for |
|---|---|---|---|
| A1 | OpenAI, Function calling guide, `developers.openai.com/api/docs/guides/function-calling` | primary | both tool definition shapes; strict mode rules; tool outputs may be an array of image and file parts; custom tools with grammars; newest models require the Responses API for tool calling |
| A2 | OpenAI, Chat Completions API reference, `developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create/` | primary | tool messages accept text only |
| A3 | OpenAI, Images and vision guide, `developers.openai.com/api/docs/guides/images-vision` | primary | accepted formats; detail levels; patch budgets |
| A4 | OpenAI, Async tool calling guide, `developers.openai.com/api/docs/guides/async-tool-calling` | primary | the model can continue while the application runs a tool |
| A5 | Anthropic, Handle tool calls, `platform.claude.com/docs/en/agents-and-tools/tool-use/handle-tool-calls` | primary | tool results with image blocks; `is_error` |
| A6 | Anthropic, Vision, `platform.claude.com/docs/en/build-with-claude/vision` | primary | size limits; resolution tiers; the 28-pixel patch formula |
| A7 | Google Cloud documentation on Claude structured outputs, `docs.cloud.google.com/gemini-enterprise-agent-platform/models/partner-models/claude/structured-outputs`; AWS blog on the same feature | secondary publishers of Anthropic's rules | strict tool use: additional properties forbidden, all properties required, numeric and length bounds unsupported |
| A8 | Google, Function calling with the Gemini API (Interactions), `ai.google.dev/gemini-api/docs/function-calling` and `…/interactions/function-calling` | primary | call and result shapes; image parts in results |
| A9 | Google, Function calling (Generate Content API, labelled Legacy), `ai.google.dev/gemini-api/docs/generate-content/function-calling` | primary | multimodal function responses for Gemini 3 series; accepted image types |
| A10 | Google Cloud, Function calling reference, `docs.cloud.google.com/gemini-enterprise-agent-platform/reference/models/function-calling` | primary | function name rule; supported schema attributes |
| A11 | Google, Image understanding, `ai.google.dev/gemini-api/docs/interactions/image-understanding` | primary | 20 MB request limit with inline data |
| A12 | Model Context Protocol, Tools, revision 2026-07-28, `modelcontextprotocol.io/specification/2026-07-28/server/tools` | primary | tool shape; content types; structured content; naming; the two kinds of error; guidance on handles |
| A13 | Model Context Protocol, Key changes, `modelcontextprotocol.io/specification/2026-07-28/changelog` | primary | sessions and handshake removed; per-request version; Tasks moved to an extension |
| A14 | Model Context Protocol, Streamable HTTP, `modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http` | primary | Origin validation; local binding; detecting older servers |
| A15 | Ollama, Tool calling, `docs.ollama.com/capabilities/tool-calling` | primary | native call and result shapes |
| A16 | Ollama, Structured outputs, `docs.ollama.com/capabilities/structured-outputs` | primary | JSON Schema in `format` |
| A17 | Ollama, Anthropic compatibility, `docs.ollama.com/api/anthropic-compatibility` | primary | a Messages-compatible endpoint exists |
| A18 | Articles citing Ollama's OpenAI compatibility page, for example `dev.to/grimicorn/ollama-in-your-dev-loop-swapping-the-base-url-is-the-easy-part-pf6` | secondary | `tool_choice` unsupported; images as base64 only |
| A19 | llama.cpp, server README, `github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md` | primary | OpenAI- and Anthropic-compatible routes; tool calling; `grammar` and `json_schema` request fields |
| A20 | Agent Skills specification, `agentskills.io/specification` | primary | `SKILL.md` front matter rules; size guidance |
| A21 | Yang et al., "Set-of-Mark Prompting Unleashes Extraordinary Visual Grounding in GPT-4V", arXiv 2310.11441 | primary | numbered marks improve region grounding |
| A22 | The rustc book, Platform support, `doc.rust-lang.org/rustc/platform-support.html` | primary | WASI targets with the standard library |
| A23 | maturin user guide, Bindings, `maturin.rs/bindings` | primary | packaging a Rust binary as a Python wheel |
| A24 | cargo-npm, `crates.io/crates/cargo-npm` | example of a pattern | per-platform optional npm packages |
| A25 | GitHub, Using artifact attestations, `docs.github.com/actions/security-for-github-actions/using-artifact-attestations/using-artifact-attestations-to-establish-provenance-for-builds` | primary | build attestation and verification |
| A26 | Guides to the DeepSeek and DashScope APIs, for example `docs.qwencloud.com/developer-guides/third-party-models/deepseek` | secondary | both accept the OpenAI chat format |
| A27 | Third-party summaries of OpenAI's structured output limits and of OpenAI's and Anthropic's tool name patterns | secondary; the figures differ between sources | property count and nesting limits, where the design stays below the smallest figure found (100 properties, 5 levels); tool names |

## Appendix B. How the measurements were made

All on commit `fddacce`, release build, 2 virtual CPUs. `V` stands for `target/release/vl`.

| Result | Command |
|---|---|
| Timings and PNG sizes | `V render examples/fruit_bowl.vl -o out.png --width W --samples S --json` |
| Check time | `V check examples/fruit_bowl.vl` |
| Stack overflow | `python3 -c "print('sphere s { radius: ' + '-'*20000 + '1 }')" \| V check - --json` |
| Quadratic check | `printf 'plane ground { }\nfor i in 0..20000 { sphere "s{i}" { radius: 0.01, at: [i * 0.03, 0.01, 0] } }\n' \| V check - --json` |
| Column counts characters | the same misspelt property after a comment containing `é` and `球`, and after an ASCII comment of equal length: both report column 43 |
| Input and output surface | `grep -rlE "std::(fs\|net\|process\|env\|io)" crates/*/src` |
| PNG text entries | reading the chunks of a rendered file |
| Package names | HTTP status of `pypi.org/pypi/NAME/json` and `registry.npmjs.org/NAME` |
| Schema and examples | `python3 docs/design/check_examples.py` (needs the `jsonschema` package) |

## Appendix C. One operation in five formats

All derived from the `RenderParams` definition in the draft schema, in the standard profile, by a throwaway script that is not part of the repository. The descriptions are drafts.

OpenAI Chat Completions, strict variant:

```json
{
  "type": "function",
  "function": {
    "name": "vl_render",
    "description": "Render a VL scene program to a picture. Returns an image id and a report measured from that picture: which objects are visible, where, how large, in which colour, and any problems found. If the program has mistakes nothing is rendered and each mistake is listed with its line and a hint. Use quality 'preview' while working and 'final' when the picture is right. A long render returns a job id: call vl_job to wait for it. Pictures are kept for 24 hours.",
    "parameters": {
      "type": "object",
      "properties": {
        "program": {"type": "string", "description": "The complete VL program text."},
        "quality": {
          "type": ["string", "null"], "enum": ["draft", "preview", "standard", "final", null],
          "description": "draft = 256 px, preview = 512 px (default), standard = 1024 px, final = the size and samples written in the program."
        },
        "region_left": {
          "type": ["number", "null"],
          "description": "Render only a part of the picture: left edge as a fraction of the width, 0 to 1."
        },
        "region_top": {
          "type": ["number", "null"],
          "description": "Top edge of the part as a fraction of the height, 0 to 1."
        },
        "region_right": {
          "type": ["number", "null"],
          "description": "Right edge of the part as a fraction of the width, 0 to 1."
        },
        "region_bottom": {
          "type": ["number", "null"],
          "description": "Bottom edge of the part as a fraction of the height, 0 to 1."
        }
      },
      "required": ["program", "quality", "region_left", "region_top", "region_right", "region_bottom"],
      "additionalProperties": false
    },
    "strict": true
  }
}
```

OpenAI Responses differs only in the envelope: `{"type": "function", "name": …, "description": …, "parameters": …, "strict": true}`.

Anthropic Messages, plain variant:

```json
{
  "name": "vl_render",
  "description": "Render a VL scene program to a picture. Returns an image id and a report measured from that picture: which objects are visible, where, how large, in which colour, and any problems found. If the program has mistakes nothing is rendered and each mistake is listed with its line and a hint. Use quality 'preview' while working and 'final' when the picture is right. A long render returns a job id: call vl_job to wait for it. Pictures are kept for 24 hours.",
  "input_schema": {
    "type": "object",
    "properties": {
      "program": {"type": "string", "description": "The complete VL program text."},
      "quality": {
        "type": "string", "enum": ["draft", "preview", "standard", "final"],
        "description": "draft = 256 px, preview = 512 px (default), standard = 1024 px, final = the size and samples written in the program."
      },
      "region_left": {
        "type": "number",
        "description": "Render only a part of the picture: left edge as a fraction of the width, 0 to 1."
      },
      "region_top": {"type": "number", "description": "Top edge of the part as a fraction of the height, 0 to 1."},
      "region_right": {"type": "number", "description": "Right edge of the part as a fraction of the width, 0 to 1."},
      "region_bottom": {"type": "number", "description": "Bottom edge of the part as a fraction of the height, 0 to 1."}
    },
    "required": ["program"]
  }
}
```

Gemini Interactions uses the same content as the Anthropic form under `{"type": "function", "name": …, "description": …, "parameters": …}`. MCP uses it under `{"name": …, "title": …, "description": …, "inputSchema": …}`.
