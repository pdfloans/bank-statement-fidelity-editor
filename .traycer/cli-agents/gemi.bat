Here are the modified templates adapted specifically for a Gemini CLI implementation.

Since AI prompts can easily exceed standard terminal character limits, I have updated the execution commands at the bottom of both scripts to utilize the temporary file (TRAYCER_PROMPT_TMP_FILE) via piping, while also passing the system prompt if your Gemini CLI supports it.

PowerShell / Windows Environment
PowerShell
REM ================================
REM Gemini CLI Agent Template (PowerShell)
REM Available environment variables:
REM   $env:TRAYCER_PROMPT - The prompt to be executed
REM   $env:TRAYCER_PROMPT_TMP_FILE - Temporary file path containing the prompt content - highly recommended for large prompts.
REM   $env:TRAYCER_TASK_ID - Traycer task identifier - use this to maintain session continuity.
REM   $env:TRAYCER_PHASE_BREAKDOWN_ID - Traycer phase breakdown identifier.
REM   $env:TRAYCER_PHASE_ID - Traycer per phase identifier.
REM   $env:TRAYCER_SYSTEM_PROMPT - System prompt to append to the CLI agent.
REM ================================

REM Example 1: Passing the prompt safely via the temp file (Recommended for large prompts)
Get-Content -Raw $env:TRAYCER_PROMPT_TMP_FILE | gemini --system "$env:TRAYCER_SYSTEM_PROMPT"

REM Example 2: Direct string passing (if you prefer standard flags and are sure limits won't be hit)
REM gemini -p "$env:TRAYCER_PROMPT" --system "$env:TRAYCER_SYSTEM_PROMPT"