#!/usr/bin/env python3
"""Check the JSON examples of the design document against the draft schema.

Every example in 01-universal-interface.md that is preceded by a line
    <!-- schema: Name -->
must be valid against  #/$defs/Name  of vlp-0.1-draft.schema.json.
A few deliberately wrong messages must be rejected, so that a schema which
accepts everything does not pass unnoticed.

Needs the `jsonschema` package (a tool for this check only; nothing shipped uses it):
    python3 -m pip install jsonschema
    python3 docs/design/check_examples.py
"""
import json
import pathlib
import re
import sys

from jsonschema import Draft202012Validator

HERE = pathlib.Path(__file__).resolve().parent
SCHEMA = json.loads((HERE / "vlp-0.1-draft.schema.json").read_text(encoding="utf-8"))
DOCUMENT = (HERE / "01-universal-interface.md").read_text(encoding="utf-8")

ENGINE = {"name": "vl", "version": "0.2.0", "render_revision": 1, "language": "1.0"}
COUNTS = {"errors": 0, "warnings": 0, "findings": 0}

# (definition, message, why it must be rejected)
MUST_BE_REJECTED = [
    ("Response", {"vlp": "0.1", "op": "check", "ok": True, "engine": ENGINE,
                  "error": {"code": "internal", "message": "x", "retryable": False}},
     "ok is true but the message carries an error and no result"),
    ("Response", {"vlp": "0.1", "op": "render", "ok": True, "engine": ENGINE,
                  "result": {"status": "done", "valid": True, "counts": COUNTS, "diagnostics": [], "diagnostics_omitted": 0}},
     "status is done but there is no image"),
    ("Response", {"vlp": "0.1", "op": "render", "ok": True, "engine": ENGINE,
                  "result": {"status": "rejected", "valid": True, "counts": COUNTS, "diagnostics": [], "diagnostics_omitted": 0}},
     "status is rejected but valid is true"),
    ("Request", {"op": "render", "params": {"program": "x", "sample": 4}},
     "unknown parameter 'sample'"),
    ("Request", {"op": "view", "params": {"image": "../../etc/passwd"}},
     "an image identifier that is a path"),
    ("Request", {"op": "check"},
     "check without parameters"),
]


def validator(name):
    if name not in SCHEMA["$defs"]:
        raise SystemExit("the schema has no definition named " + name)
    return Draft202012Validator({"$schema": SCHEMA["$schema"], "$ref": "#/$defs/" + name, "$defs": SCHEMA["$defs"]})


def main():
    Draft202012Validator.check_schema(SCHEMA)
    problems = 0
    examples = re.findall(r"<!-- schema: (\w+) -->\s*```json\n(.*?)\n```", DOCUMENT, flags=re.S)
    if not examples:
        raise SystemExit("no examples found in the document")
    for number, (name, text) in enumerate(examples, 1):
        try:
            message = json.loads(text)
        except json.JSONDecodeError as error:
            print("FAIL example %d (%s): not JSON: %s" % (number, name, error))
            problems += 1
            continue
        errors = sorted(validator(name).iter_errors(message), key=lambda e: list(e.path))
        label = message.get("op", "") + (" request" if "params" in message or name == "Request" else " response")
        print("%s example %2d: %-22s as %s" % ("FAIL" if errors else "ok  ", number, label, name))
        for error in errors:
            print("       at %s: %s" % ("/".join(str(p) for p in error.path) or "(root)", error.message[:200]))
        problems += bool(errors)
    for name, message, why in MUST_BE_REJECTED:
        rejected = any(True for _ in validator(name).iter_errors(message))
        print("%s wrong message rejected: %s" % ("ok  " if rejected else "FAIL", why))
        problems += not rejected
    print("%d examples, %d wrong messages, %d problems" % (len(examples), len(MUST_BE_REJECTED), problems))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
