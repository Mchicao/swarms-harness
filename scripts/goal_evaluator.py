import sys
import os
import argparse
import json
import time
from pathlib import Path

# Add parent dir to sys.path to resolve core packages
sys.path.append(os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

def evaluate_goal(goal_text, log_content, diff_content):
    """
    Evaluates whether the agent has achieved the specified goal using
    Antigravity CLI (Gemini 3.5 Flash) via the agy_call.py wrapper.

    NOTE: `agy --print` does not emit the model response on stdout in headless
    mode (see docs/AGY_PROGRAMMATIC.md). We use agy_complete(), which reads the
    answer from the persisted conversation instead.
    """
    from scripts.agy_call import agy_complete

    # Prompt for evaluation
    prompt = f"""
You are an independent QA Evaluator.
Your goal is to evaluate if the agent successfully completed the user's objective based on the changes diff and terminal logs.

USER GOAL:
{goal_text}

TERMINAL LOGS:
{log_content}

CHANGES DIFF:
{diff_content}

INSTRUCTIONS:
Determine if the goal is fully achieved. Output ONLY a valid JSON block with these keys:
- "done": true or false
- "reason": A brief explanation of why the goal is met or what is missing.

RESPONSE FORMAT:
{{"done": true, "reason": "All pytest passes successfully"}}
"""

    model = os.environ.get("AGY_MODEL", "Gemini 3.5 Flash (Medium)")
    try:
        output = agy_complete(prompt, model=model, timeout=180)

        # Parse and log token telemetry
        try:
            from scripts.utils.token_telemetry import parse_stdout_text, record_event
            usage = parse_stdout_text(output)
            benchmark_id = os.environ.get("SWARM_BENCHMARK_ID", "default-run")
            run_id = os.environ.get("SWARM_RUN_ID", "default-run")
            record_event(
                run_id=run_id,
                benchmark_id=benchmark_id,
                phase="goal_eval",
                provider="antigravity_cli",
                model="gemini-3.5-flash",
                role="overhead",
                task_id=goal_text[:30],
                input_tokens=usage["input"],
                cache_read_tokens=usage["cached"],
                output_tokens=usage["output"],
                reasoning_tokens=usage["reasoning"],
                usage_source="cli_reported" if usage["input"] > 0 else "missing",
                success=True,
            )
        except Exception:
            pass

        # Find JSON block in output
        if "{" in output and "}" in output:
            json_str = output[output.find("{"):output.rfind("}")+1]
            data = json.loads(json_str)
            return data.get("done", False), data.get("reason", "No reason provided")

        return False, f"Could not parse evaluator JSON response. Raw output: {output[:200]}"
    except Exception as e:
        return False, f"Evaluator execution error: {str(e)}"

def main():
    parser = argparse.ArgumentParser(description="Swarm Goal Evaluator")
    parser.add_argument("--goal", required=True, help="Goal condition text")
    parser.add_argument("--log", required=True, help="Path to worker log file")
    parser.add_argument("--diff", required=True, help="Path to git diff file")
    args = parser.parse_args()
    
    log_path = Path(args.log)
    diff_path = Path(args.diff)
    
    log_content = ""
    if log_path.exists():
        log_content = log_path.read_text(encoding="utf-8", errors="replace")[-5000:] # Last 5k chars
        
    diff_content = ""
    if diff_path.exists():
        diff_content = diff_path.read_text(encoding="utf-8", errors="replace")[:5000] # First 5k chars
        
    done, reason = evaluate_goal(args.goal, log_content, diff_content)
    
    print(json.dumps({"done": done, "reason": reason}))
    sys.exit(0 if done else 1)

if __name__ == "__main__":
    main()
