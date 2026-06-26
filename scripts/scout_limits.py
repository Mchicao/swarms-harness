#!/usr/bin/env python3
"""
Scout Limits - Rate Limit Probing for Swarm V11.5
Valida disponibilidad y latencia de Gemini, OpenRouter y Groq antes de lanzar el enjambre.
Genera 'swarm_limits.yaml' con el 'max_safe_concurrency' recomendado.
"""

import asyncio
import json
import os
import time
from pathlib import Path

import yaml

# Colores para output
GREEN = "\033[92m"
YELLOW = "\033[93m"
RED = "\033[91m"
RESET = "\033[0m"


async def check_antigravity(is_api_key: bool):
    """Prueba simple contra Antigravity (CLI o API Key)."""
    mode = "API_KEY" if is_api_key else "CLI_AUTH"
    
    # Configure environment if is_api_key is set
    env = os.environ.copy()
    if is_api_key:
        api_key = env.get("GOOGLE_API_KEY")
        if not api_key:
            return mode, False, 0.0, "Missing GOOGLE_API_KEY"
    
    # Check agy version to verify it is installed and executable
    cmd = ["agy", "--version"]

    start = time.time()
    try:
        if os.name == "nt":
            cmd_str = " ".join(cmd)
            proc = await asyncio.create_subprocess_shell(
                cmd_str, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE, env=env
            )
        else:
            proc = await asyncio.create_subprocess_exec(
                *cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE, env=env
            )
        stdout, stderr = await proc.communicate()
        duration = time.time() - start

        if proc.returncode == 0:
            return mode, True, duration, "OK"
        else:
            err = stderr.decode().strip() or stdout.decode().strip()
            return mode, False, duration, f"Error: {err[:100]}"
    except Exception as e:
        return mode, False, 0.0, str(e)


async def check_provider_http(provider: str, model: str, api_key_env: str, url: str):
    """Prueba HTTP genérica para OpenRouter/Groq."""
    import aiohttp

    api_key = os.environ.get(api_key_env)
    if not api_key:
        return provider, False, 0.0, f"Missing {api_key_env}"

    headers = {"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"}

    # OpenRouter specific headers
    if provider == "OPENROUTER":
        headers["HTTP-Referer"] = "https://antigravity.swarm"
        headers["X-Title"] = "Antigravity Swarm"

    payload = {"model": model, "messages": [{"role": "user", "content": "Hi"}], "max_tokens": 5}

    start = time.time()
    try:
        async with aiohttp.ClientSession() as session:
            async with session.post(url, headers=headers, json=payload, timeout=10) as resp:
                duration = time.time() - start
                if resp.status == 200:
                    return provider, True, duration, "OK"
                elif resp.status == 429:
                    return provider, False, duration, "429 Rate Limit"
                else:
                    text = await resp.text()
                    return provider, False, duration, f"HTTP {resp.status}: {text[:50]}"
    except Exception as e:
        return provider, False, 0.0, str(e)


async def stress_test(provider, func, max_workers=5):
    """Ejecuta ráfagas incrementales para encontrar el límite."""
    print(f"\n[STRESS] Stress Testing {provider}...")
    safe_concurrency = 0

    # Ramp up: 1, 3, 5, 10
    levels = [1, 3, 5]
    if provider == "OPENROUTER":
        levels = [1, 2]  # OpenRouter free is very limited

    for count in levels:
        print(f"   Burst: {count} reqs...", end="", flush=True)
        tasks = [func() for _ in range(count)]
        results = await asyncio.gather(*tasks)

        successes = sum(1 for r in results if r[1])
        avg_lat = sum(r[2] for r in results) / count if count > 0 else 0

        if successes == count:
            print(f" {GREEN}PASS{RESET} (Avg: {avg_lat:.2f}s)")
            safe_concurrency = count
        else:
            errors = [r[3] for r in results if not r[1]]
            print(f" {RED}FAIL{RESET} ({successes}/{count} ok) - Errors: {errors}")
            break

        # Cooldown inteligente
        cooldown = 4 if provider == "GROQ" else 1
        await asyncio.sleep(cooldown)

    return safe_concurrency


async def check_all(args):
    """Ejecuta los checks según el modo."""
    limits = {}

    # 1. Check Antigravity Dual Channel
    if args.stress:
        cli_safe = await stress_test("Antigravity CLI", lambda: check_antigravity(False))
        api_safe = await stress_test("Antigravity API", lambda: check_antigravity(True))
    else:
        res_cli = await check_antigravity(False)
        res_api = await check_antigravity(True)
        print(f"Antigravity CLI: {res_cli}")
        print(f"Antigravity API: {res_api}")
        cli_safe = 1 if res_cli[1] else 0
        api_safe = 1 if res_api[1] else 0

    limits["antigravity_cli"] = {
        "status": "ok" if cli_safe > 0 else "down",
        "max_safe_concurrency": cli_safe,
    }
    limits["antigravity_api"] = {
        "status": "ok" if api_safe > 0 else "down",
        "max_safe_concurrency": api_safe,
    }

    # 2. Check OpenRouter
    or_models = {
        "or_gemma4": "google/gemma-4-31b-it:free",
        "or_llama33": "meta-llama/llama-3.3-70b-instruct:free",
        "or_qwen3_coder": "qwen/qwen3-coder:free",
        "or_free_router": "openrouter/free",
    }

    for key, model_id in or_models.items():
        checker = lambda m=model_id: check_provider_http(
            "OPENROUTER",
            m,
            "OPENROUTER_API_KEY",
            "https://openrouter.ai/api/v1/chat/completions",
        )
        if args.stress:
            safe = await stress_test(f"OpenRouter ({model_id})", checker)
        else:
            res = await checker()
            print(f"{key}: {res}")
            safe = 1 if res[1] else 0

        limits[key] = {
            "status": "ok" if safe > 0 else "down",
            "max_safe_concurrency": safe,
            "model": model_id,
        }

    # 3. Check Kilo CLI Models (free tier)
    kilo_models = {
        "kilo_nex": "kilo/nex-agi/nex-n2-pro:free",
        "kilo_step_flash": "kilo/stepfun/step-3.7-flash:free",
        "kilo_laguna": "kilo/poolside/laguna-m.1:free",
        "kilo_auto_free": "kilo/kilo-auto/free",
    }

    for key, model_id in kilo_models.items():
        t0 = time.time()
        try:
            cmd = ["kilo", "run", "-m", model_id, "--auto", "Respond only: OK"]
            if os.name == "nt":
                proc = await asyncio.create_subprocess_shell(
                    " ".join(cmd),
                    stdout=asyncio.subprocess.PIPE,
                    stderr=asyncio.subprocess.PIPE
                )
            else:
                proc = await asyncio.create_subprocess_exec(
                    *cmd,
                    stdout=asyncio.subprocess.PIPE,
                    stderr=asyncio.subprocess.PIPE
                )
            
            try:
                stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=5.0)
                latency = round((time.time() - t0) * 1000)
                out_str = stdout.decode().strip() or stderr.decode().strip()
                is_ok = proc.returncode == 0 and "OK" in out_str
                print(f"Kilo ({model_id}): {'OK' if is_ok else 'FAIL'} ({latency}ms)")
                limits[key] = {
                    "status": "ok" if is_ok else "down",
                    "max_safe_concurrency": 3 if is_ok else 0,
                    "latency_ms": latency,
                    "model": model_id,
                }
            except asyncio.TimeoutError:
                try:
                    proc.kill()
                except:
                    pass
                print(f"Kilo ({model_id}): TIMEOUT")
                limits[key] = {
                    "status": "down",
                    "max_safe_concurrency": 0,
                    "model": model_id,
                }
        except Exception as e:
            print(f"Kilo ({model_id}): ERROR - {e}")
            limits[key] = {
                "status": "down",
                "max_safe_concurrency": 0,
                "model": model_id,
            }

    # 4. Meta & Strategy
    total_cap = sum(v["max_safe_concurrency"] for k, v in limits.items())
    limits["_meta"] = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "total_capacity": total_cap,
        "recommended_strategy": "role-based" if total_cap > 3 else "round-robin",
        "recommended_workers": min(total_cap, 6) if total_cap > 0 else 1,
    }

    return limits


def load_env():
    """Carga variables desde el archivo .env si existe."""
    env_file = Path(".env")
    if env_file.exists():
        with open(env_file, "r", encoding="utf-8") as f:
            for line in f:
                if "=" in line and not line.strip().startswith("#"):
                    k, v = line.split("=", 1)
                    os.environ[k.strip()] = v.strip().strip("'\"")


async def main():
    import argparse

    load_env()
    parser = argparse.ArgumentParser()
    parser.add_argument("--stress", action="store_true", help="Ejecutar prueba de carga")
    args = parser.parse_args()

    limits = await check_all(args)

    # Save limits
    output = Path(".agent/swarm_limits.yaml")
    with open(output, "w") as f:
        yaml.dump(limits, f)

    print(f"\n[OK] Límites guardados en {output}")
    print(json.dumps(limits, indent=2))


if __name__ == "__main__":
    # Check dependencies
    try:
        import aiohttp
    except ImportError:
        print("Installing dependencies...")
        os.system("uv pip install aiohttp pyyaml")
        print("Done.")

    asyncio.run(main())
