# The verified-authorship benchmark ("vibe coding" under contract)

Measures whether an AI can author a correct Rastok graph when the
compiler and the Lab hold the contract: the model writes, `rastok
check` refuses or accepts, the referee's simulation decides whether
the behavior — not just the syntax — is right. The experiment
arbitrates between three adoption surfaces: direct generation,
guided generation, or an expert-only authoring surface.

## Layout

- `prompts/` — the language card given to every model, one frozen
  file per version (`v1.md` = the M0 surface). A card is never
  edited after a wave has used it; a language evolution gets a new
  version, and re-running old tasks on a new card measures stability.
- `tasks/T*/` — one directory per task:
  - `task.md` — the statement the model receives (with the pinned
    interface names the referee depends on);
  - `referee.rkg` — the acceptance lab, never shown to the model;
  - `reference.rkg` — our own solution proving the referee is
    satisfiable (also: it must stay green — a referee no reference
    satisfies is a broken referee).
- `runner/` — the loop: generate → check → feed diagnostics back →
  retry → simulate → feed scenario failures back → retry. Bounded
  attempts. Every attempt and every piece of feedback is kept on
  disk for audit.
- `results/` — run artifacts and the metrics JSONL, git-ignored:
  measures are read and interpreted on the strategy side.

## The loop, per task and per author

1. The author receives the language card and `task.md`, and returns
   one `.rkg` source (attempt 1).
2. `rastok check <attempt>` — refusals are returned to the author
   verbatim as feedback, and the attempt counter increments.
3. On a clean check, `rastok simulate <attempt> referee.rkg` —
   scenario failures likewise feed back.
4. Stop at referee-green or at the attempt cap (default 4).

Authors are pluggable: `canned:<dir>` replays pre-recorded attempts
(the dry mode — no API spend; also how the runner tests itself),
`command:<name>:<template>` pipes the prompt to any model CLI — the
NAME is the recorded identity, so several models in one wave stay
distinguishable in the metrics and in their work directories. On a
retry, the author receives the full transcript: each prior attempt's
SOURCE with the feedback it earned — a corrector that cannot see
what it must correct measures nothing. The runner's exit code tells
the truth: 0 only when at least one task passed, 1 when tasks ran
and none passed, 2 when nothing was measured at all.

## Metrics (one JSONL row per author × task)

Every row carries its identity — `run_id`, `wave`, `author` (the
model name), `prompt` version and `prompt_hash`, `task` and
`task_hash` — so two models, two waves, or two prompt revisions are
never conflated. Then the measures: `first_try_check` and
`first_try_scenarios` (the first-pass rates), `attempts` (iterations
to a valid graph), `outcome`, `codes` (every diagnostic code seen —
the recurring-error families), and `human_minutes` (null until a
human had to intervene; filled by hand). Wave-over-wave comparison
of the same tasks under a new prompt version measures stability
under language evolution.

## Validating the referees

Every reference solution must stay green against its referee:

    for t in tasks/T*/; do
      cargo run -q -p rastok -- simulate $t/reference.rkg $t/referee.rkg
    done
