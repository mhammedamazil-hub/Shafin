#!/usr/bin/env python3
"""Probe which text LLMs are reachable from a GitHub-hosted runner via GitHub Models.

The development sandbox cannot reach any LLM endpoint, so multi-model experiments
run on a runner (workflow vl-llm.yml). This probe records, without guessing:
the model catalogue, which models answer with the Actions token, their limits.
Python standard library only.
"""
import json
import os
import sys
import time
import urllib.error
import urllib.request

BASE = "https://models.github.ai"
TOKEN = os.environ.get("GITHUB_TOKEN", "")


def call(method, path, body=None, timeout=120):
    req = urllib.request.Request(BASE + path, method=method)
    req.add_header("Authorization", f"Bearer {TOKEN}")
    req.add_header("Accept", "application/vnd.github+json")
    req.add_header("X-GitHub-Api-Version", "2022-11-28")
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        req.add_header("Content-Type", "application/json")
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, data=data, timeout=timeout) as r:
            raw = r.read().decode("utf-8", "replace")
            return {"status": r.status, "headers": dict(r.headers), "body": raw, "seconds": round(time.time() - t0, 2)}
    except urllib.error.HTTPError as e:
        return {"status": e.code, "headers": dict(e.headers), "body": e.read().decode("utf-8", "replace")[:2000],
                "seconds": round(time.time() - t0, 2)}
    except Exception as e:  # noqa: BLE001
        return {"status": 0, "headers": {}, "body": f"{type(e).__name__}: {e}", "seconds": round(time.time() - t0, 2)}


def main():
    req = json.load(open(sys.argv[1]))
    out_dir = sys.argv[2]
    os.makedirs(out_dir, exist_ok=True)
    result = {"request": req, "catalog_status": None, "models": [], "tests": []}

    cat = call("GET", "/catalog/models")
    result["catalog_status"] = cat["status"]
    result["catalog_raw_head"] = cat["body"][:600]
    result["catalog_headers"] = cat["headers"]
    result["token_len"] = len(TOKEN)
    models = []
    if cat["status"] == 200:
        try:
            models = json.loads(cat["body"])
        except Exception as e:  # noqa: BLE001
            result["catalog_error"] = str(e)
    else:
        result["catalog_error"] = cat["body"][:500]
    for m in models:
        result["models"].append({
            "id": m.get("id"), "publisher": m.get("publisher"), "rate_limit_tier": m.get("rate_limit_tier"),
            "in": m.get("supported_input_modalities"), "out": m.get("supported_output_modalities"),
            "limits": m.get("limits"), "capabilities": m.get("capabilities"), "tags": m.get("tags"),
        })
    print(f"catalog: status={cat['status']} models={len(models)}", flush=True)

    wanted = req.get("test_models", [])
    for mid in wanted:
        body = {"model": mid, "messages": [{"role": "user", "content": req.get("prompt", "Reply with the single word: ok")}],
                "max_tokens": 16}
        r = call("POST", "/inference/chat/completions", body)
        text, usage = None, None
        if r["status"] == 200:
            try:
                j = json.loads(r["body"])
                text = j["choices"][0]["message"].get("content")
                usage = j.get("usage")
            except Exception as e:  # noqa: BLE001
                text = f"<unparseable: {e}>"
        rl = {k: v for k, v in r["headers"].items() if "ratelimit" in k.lower() or k.lower() == "retry-after"}
        entry = {"model": mid, "status": r["status"], "seconds": r["seconds"], "text": text, "usage": usage,
                 "ratelimit": rl, "error": None if r["status"] == 200 else r["body"][:400],
                 "raw_head": r["body"][:300], "headers": r["headers"]}
        result["tests"].append(entry)
        print(f"  {mid}: status={r['status']} text={text!r} {rl}", flush=True)
        time.sleep(float(req.get("pause", 4)))

    json.dump(result, open(os.path.join(out_dir, "probe.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
