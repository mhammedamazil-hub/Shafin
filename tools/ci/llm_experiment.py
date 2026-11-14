#!/usr/bin/env python3
"""Can text-only language models write VL? A small, fully logged experiment.

For every (model, scene description) pair the model is given the one-page language
reference and asked for a VL program. The program goes through the real tool:

    round 0:   write            -> vl check
    round k:   repair from the tool's diagnostics, or revise from `vl inspect`

Every round is recorded verbatim (reply, program, diagnostics, inspection), the last
valid program is rendered at low resolution, and everything is written to OUT_DIR.
Nothing is filtered or retried selectively: failures are data.

Backends: Ollama on this machine (open-weight models, CPU) and, optionally, an
OpenAI-compatible HTTP endpoint. Python standard library only.
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

OLLAMA = "http://127.0.0.1:11434"
T0 = time.time()


def http_json(url, body, timeout):
    req = urllib.request.Request(url, data=json.dumps(body).encode(), method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("User-Agent", "vl-experiment")
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read().decode("utf-8", "replace"), time.time() - t0
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")[:2000], time.time() - t0
    except Exception as e:  # noqa: BLE001
        return 0, f"{type(e).__name__}: {e}", time.time() - t0


def chat(backend, messages, num_predict):
    """Returns (text or None, meta)."""
    if backend["type"] == "ollama":
        body = {"model": backend["model"], "messages": messages, "stream": False, "keep_alive": "20m",
                "options": {"temperature": 0.2, "seed": 1, "num_ctx": 8192, "num_predict": num_predict}}
        status, text, secs = http_json(OLLAMA + "/api/chat", body, 1500)
        meta = {"status": status, "seconds": round(secs, 1)}
        if status != 200:
            meta["error"] = text[:500]
            return None, meta
        j = json.loads(text)
        meta.update({"prompt_tokens": j.get("prompt_eval_count"), "output_tokens": j.get("eval_count"),
                     "prompt_eval_seconds": round(j.get("prompt_eval_duration", 0) / 1e9, 1),
                     "generation_seconds": round(j.get("eval_duration", 0) / 1e9, 1)})
        return j.get("message", {}).get("content", ""), meta
    # OpenAI-compatible endpoint (reasoning models need room for their hidden reasoning)
    body = {"model": backend["model"], "messages": messages, "temperature": 0.2, "seed": 1, "max_tokens": num_predict + 2500}
    meta = {}
    for attempt in range(3):
        status, text, secs = http_json(backend["url"], body, 240)
        meta = {"status": status, "seconds": round(secs, 1), "attempts": attempt + 1}
        if status == 200:
            try:
                j = json.loads(text)
                meta.update({"served_model": j.get("model"), "usage": j.get("usage")})
                return j["choices"][0]["message"].get("content") or "", meta
            except Exception as e:  # noqa: BLE001
                meta["error"] = f"unparseable response: {e}: {text[:300]}"
        else:
            meta["error"] = text[:500]
        time.sleep(20 * (attempt + 1))
    return None, meta


def extract_program(reply):
    blocks = re.findall(r"```[a-zA-Z0-9_+-]*[ \t]*\n(.*?)```", reply, re.S)
    if blocks:
        return max(blocks, key=len).strip() + "\n"
    # an unterminated block (the model ran out of tokens)
    m = re.search(r"```[a-zA-Z0-9_+-]*[ \t]*\n(.*)$", reply, re.S)
    if m:
        return m.group(1).strip() + "\n"
    return reply.strip() + "\n"


def run(cmd, timeout=600):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return 124, "", "timeout"


def findings_of(inspect_text):
    out, on = [], False
    for line in inspect_text.splitlines():
        if line.startswith("visual findings:"):
            on = "none" not in line
            continue
        if on and line.startswith("  "):
            out.append(line.strip())
    return out


def main():
    req = json.load(open(sys.argv[1]))
    out_dir = sys.argv[2]
    vl = os.path.abspath(req.get("vl_binary", "target/release/vl"))
    reference = open(req.get("reference", "skills/vl-scene/references/language.md")).read()
    system = ("You write programs in VL, a small language that describes 3D scenes; a tool renders them into images. "
              "Reply with ONE complete VL program inside a ```vl code block and nothing else. "
              "Use only what this reference describes.\n\n" + reference)
    budget = req.get("time_budget_minutes", 30) * 60
    max_rounds = req.get("max_rounds", 3)
    num_predict = req.get("num_predict", 700)
    rw, rs = req.get("render", {}).get("width", 480), req.get("render", {}).get("samples", 24)
    results = {"request": req, "reference_words": len(reference.split()), "runs": [], "vl_version": run([vl, "version"])[1].strip()}

    def save():
        results["elapsed_minutes"] = round((time.time() - T0) / 60, 1)
        json.dump(results, open(os.path.join(out_dir, "results.json"), "w"), indent=1)

    os.makedirs(out_dir, exist_ok=True)
    for bi, backend in enumerate(req["backends"]):
        tag = re.sub(r"[^a-zA-Z0-9_.-]+", "_", backend["name"])
        if backend["type"] == "ollama":
            rc, out, err = run(["ollama", "pull", backend["model"]], 1500)
            if rc != 0:
                results["runs"].append({"backend": backend["name"], "status": "model could not be pulled", "detail": (out + err)[-400:]})
                save()
                continue
        for prompt in req["prompts"]:
            rec = {"backend": backend["name"], "prompt": prompt["id"], "rounds": [], "status": "not run"}
            results["runs"].append(rec)
            if time.time() - T0 > budget:
                rec["status"] = "skipped: time budget exhausted"
                save()
                continue
            d = os.path.join(out_dir, tag, prompt["id"])
            os.makedirs(d, exist_ok=True)
            messages = [{"role": "system", "content": system},
                        {"role": "user", "content": "Write a VL program for this scene:\n" + prompt["text"]}]
            last_valid = None
            try:
                for k in range(max_rounds):
                    if backend["type"] != "ollama" and (k > 0 or prompt is not req["prompts"][0]):
                        time.sleep(req.get("hosted_pause_seconds", 16))
                    reply, meta = chat(backend, messages, num_predict)
                    rnd = {"round": k, "llm": meta}
                    rec["rounds"].append(rnd)
                    if reply is None:
                        rnd["outcome"] = "no reply from the model"
                        break
                    open(os.path.join(d, f"round{k}.reply.txt"), "w").write(reply)
                    if reply.strip().strip(".").upper() == "DONE":
                        rnd["outcome"] = "model declared the picture finished"
                        break
                    path = os.path.join(d, f"round{k}.vl")
                    open(path, "w").write(extract_program(reply))
                    rc, out, err = run([vl, "check", path, "--json"])
                    try:
                        cj = json.loads(out)
                    except Exception:  # noqa: BLE001
                        cj = {"ok": False, "errors": -1, "warnings": 0, "diagnostics": []}
                        rnd["tool_failure"] = (out + err)[-400:]
                    rnd.update({"valid": bool(cj.get("ok")), "errors": cj.get("errors"), "warnings": cj.get("warnings"),
                                "codes": [x.get("code") for x in cj.get("diagnostics", [])]})
                    _, _, human = run([vl, "check", path])
                    open(os.path.join(d, f"round{k}.check.txt"), "w").write(human)
                    if not cj.get("ok"):
                        rnd["outcome"] = "errors"
                        feedback = ("The tool rejected the program:\n" + human[:2200] +
                                    "\nReply with the complete corrected program in a ```vl block.")
                    else:
                        last_valid = path
                        rc, itext, ierr = run([vl, "inspect", path])
                        open(os.path.join(d, f"round{k}.inspect.txt"), "w").write(itext)
                        f = findings_of(itext)
                        rnd["visual_findings"] = [x.split(" ")[0] for x in f]
                        rnd["outcome"] = "valid"
                        if not f and not cj.get("warnings"):
                            rnd["outcome"] = "valid, nothing reported"
                            break
                        feedback = ("The program is valid. This is the picture it produces, as measured by the tool:\n" +
                                    itext[:2600] + ("\nCompiler warnings:\n" + human[:900] if cj.get("warnings") else "") +
                                    "\nIf something is not as the scene description asks, reply with the complete improved "
                                    "program in a ```vl block. If the picture is right, reply with exactly: DONE")
                    messages += [{"role": "assistant", "content": reply}, {"role": "user", "content": feedback}]
                rec["status"] = "valid program" if last_valid else "no valid program"
                rec["first_attempt_valid"] = bool(rec["rounds"] and rec["rounds"][0].get("valid"))
                rec["rounds_used"] = len(rec["rounds"])
                if last_valid:
                    rec["final_program"] = os.path.relpath(last_valid, out_dir)
                    png = os.path.join(d, "final.png")
                    rc, out, err = run([vl, "render", last_valid, "-o", png, "--width", str(rw), "--samples", str(rs), "--json"], 900)
                    try:
                        rj = json.loads(out)
                        rec["render"] = rj.get("render")
                        rec["final_findings"] = [x.get("code") for x in rj.get("inspection", {}).get("visual_findings", [])]
                        rec["final_warnings"] = [x.get("code") for x in rj.get("diagnostics", [])]
                    except Exception:  # noqa: BLE001
                        rec["render_error"] = (out + err)[-400:]
            except Exception as e:  # noqa: BLE001
                rec["status"] = f"harness error: {type(e).__name__}: {e}"
            save()
            print(json.dumps({k: v for k, v in rec.items() if k != "rounds"} | {"round_outcomes": [r.get("outcome") for r in rec["rounds"]]}), flush=True)
        if backend["type"] == "ollama":
            run(["ollama", "rm", backend["model"]])
    save()
    done = [r for r in results["runs"] if r.get("status") in ("valid program", "no valid program")]
    print(f"finished: {len(done)} of {len(results['runs'])} runs completed in {results['elapsed_minutes']} minutes", flush=True)
    return 0 if done else 2


if __name__ == "__main__":
    sys.exit(main())
