#!/usr/bin/env python3
"""Probe which open-weight text LLMs a GitHub-hosted CPU runner can run locally.

Background (measured, see docs/research/00-source-notes.md): GitHub Models was
retired on 2026-07-30, so there is no token-free hosted LLM for CI any more, and
the development sandbox has no route to any LLM API. Open-weight models served
by Ollama on the runner's CPU need no account or key and match the project's
"local text LLM, CPU only" constraint.

For each candidate tag: pull, run one fixed chat request through the
OpenAI-compatible endpoint, record size and tokens/second, remove the model.
Python standard library only.
"""
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

OLLAMA = "http://127.0.0.1:11434"


def post(url, body, timeout=1500, headers=None):
    req = urllib.request.Request(url, data=json.dumps(body).encode(), method="POST")
    req.add_header("Content-Type", "application/json")
    for k, v in (headers or {}).items():
        req.add_header(k, v)
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read().decode("utf-8", "replace"), round(time.time() - t0, 2)
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")[:1000], round(time.time() - t0, 2)
    except Exception as e:  # noqa: BLE001
        return 0, f"{type(e).__name__}: {e}", round(time.time() - t0, 2)


def sh(cmd, timeout=1800):
    t0 = time.time()
    try:
        p = subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=timeout)
        return p.returncode, (p.stdout + p.stderr)[-1500:], round(time.time() - t0, 1)
    except subprocess.TimeoutExpired:
        return 124, "timeout", round(time.time() - t0, 1)


PROMPT_PAD = ("You will be given the reference of a small scene description language.\n" +
              "\n".join(f"rule {i}: property p{i} takes a number between 0 and {i + 1}." for i in range(160)))
QUESTION = "Write a haiku about a red sphere on a wooden table. Then list three physical properties of glass as JSON."


def main():
    req = json.load(open(sys.argv[1]))
    out_dir = sys.argv[2]
    os.makedirs(out_dir, exist_ok=True)
    result = {"request": req, "models": [], "keyless_hosted": []}

    rc, out, secs = sh("ollama --version")
    result["ollama_version"] = out.strip()
    for tag in req.get("ollama_models", []):
        entry = {"tag": tag}
        rc, out, secs = sh(f"ollama pull {tag}", timeout=1500)
        entry.update({"pull_rc": rc, "pull_seconds": secs})
        if rc != 0:
            entry["pull_error"] = out[-400:]
            result["models"].append(entry)
            print(f"{tag}: pull failed rc={rc}", flush=True)
            continue
        rc2, show, _ = sh(f"ollama show {tag}")
        entry["show"] = show[-900:]
        rc3, lst, _ = sh("ollama list")
        entry["list"] = [l for l in lst.splitlines() if tag.split(':')[0] in l][:2]
        body = {"model": tag, "stream": False, "options": {"temperature": 0, "seed": 1, "num_ctx": 8192, "num_predict": 160},
                "messages": [{"role": "system", "content": PROMPT_PAD}, {"role": "user", "content": QUESTION}]}
        status, text, secs = post(OLLAMA + "/api/chat", body)
        entry["chat_status"] = status
        entry["chat_seconds"] = secs
        if status == 200:
            j = json.loads(text)
            pe, pd = j.get("prompt_eval_count", 0), j.get("prompt_eval_duration", 1)
            ec, ed = j.get("eval_count", 0), j.get("eval_duration", 1)
            entry.update({"prompt_tokens": pe, "prompt_tok_per_s": round(pe / (pd / 1e9), 1) if pd else None,
                          "gen_tokens": ec, "gen_tok_per_s": round(ec / (ed / 1e9), 1) if ed else None,
                          "load_seconds": round(j.get("load_duration", 0) / 1e9, 1),
                          "reply_head": j.get("message", {}).get("content", "")[:300]})
            # second request with the same long prefix: measures prompt-cache benefit
            body["messages"][1]["content"] = "Name two metals. One line."
            body["options"]["num_predict"] = 30
            status, text, secs = post(OLLAMA + "/api/chat", body)
            if status == 200:
                j = json.loads(text)
                entry["second_request_seconds"] = secs
                entry["second_prompt_eval_count"] = j.get("prompt_eval_count")
            # OpenAI-compatible endpoint check
            status, text, secs = post(OLLAMA + "/v1/chat/completions", {"model": tag, "max_tokens": 8, "temperature": 0,
                                                                         "messages": [{"role": "user", "content": "Say ok"}]})
            entry["openai_compat_status"] = status
        else:
            entry["chat_error"] = text[:400]
        sh(f"ollama rm {tag}")
        result["models"].append(entry)
        print(json.dumps({k: v for k, v in entry.items() if k not in ("show", "reply_head", "list")}), flush=True)
        json.dump(result, open(os.path.join(out_dir, "local_llm_probe.json"), "w"), indent=1)

    for ep in req.get("keyless_endpoints", []):
        status, text, secs = post(ep["url"], {"model": ep.get("model", "openai"), "max_tokens": 20,
                                              "messages": [{"role": "user", "content": "Reply with the single word: ok"}]}, timeout=90)
        item = {"url": ep["url"], "model": ep.get("model"), "status": status, "seconds": secs, "body_head": text[:500]}
        print(f"keyless {ep['url']}: {status} {text[:120]!r}", flush=True)
        # a request of realistic size (long system prompt, a few hundred output tokens)
        status, text, secs = post(ep["url"], {"model": ep.get("model", "openai"), "max_tokens": 400, "temperature": 0, "seed": 1,
                                              "messages": [{"role": "system", "content": PROMPT_PAD}, {"role": "user", "content": QUESTION}]}, timeout=180)
        item.update({"long_status": status, "long_seconds": secs, "long_body_head": text[:1200]})
        print(f"keyless long {ep['url']}: {status} in {secs}s", flush=True)
        result["keyless_hosted"].append(item)
    result["keyless_model_lists"] = []
    for url in req.get("keyless_model_lists", []):
        t0 = time.time()
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "vl-probe"}), timeout=60) as r:
                status, text = r.status, r.read().decode("utf-8", "replace")
        except urllib.error.HTTPError as e:
            status, text = e.code, e.read().decode("utf-8", "replace")
        except Exception as e:  # noqa: BLE001
            status, text = 0, f"{type(e).__name__}: {e}"
        result["keyless_model_lists"].append({"url": url, "status": status, "seconds": round(time.time() - t0, 2), "body_head": text[:6000]})
        print(f"model list {url}: {status} ({len(text)} bytes)", flush=True)

    rc, out, _ = sh("nproc; free -m | head -2; df -h / | tail -1; lscpu | grep -E 'Model name|Flags' | cut -c1-400")
    result["runner"] = out
    working = [m["tag"] for m in result["models"] if m.get("chat_status") == 200]
    result["working_local_models"] = working
    json.dump(result, open(os.path.join(out_dir, "local_llm_probe.json"), "w"), indent=1)
    if req.get("ollama_models") and not working:
        print("FAIL: no local model answered a chat request", flush=True)
        return 2
    print(f"OK: {len(working)} of {len(req.get('ollama_models', []))} local models answered", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
