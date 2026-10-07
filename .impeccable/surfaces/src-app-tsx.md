---
version: 1
slug: "src-app-tsx"
primary_target: "src/App.tsx"
related_targets: ["src/views/InicioView.tsx","src/views/HistoricoView.tsx","src/views/InsightsView.tsx","src/views/ConfiguracoesView.tsx"]
---

## Scope and mode

Replacement visual world for the complete Sonora desktop main window (shell, Início, Histórico, Insights, Ajustes). The floating gadget keeps its form and only aligns to the new tokens. Mode: Operate. Light and dark themes, following Windows by default.

## Audience and job

Windows users dictate into other applications throughout the day, then open Sonora to find a past dictation, recover a failure, or tune pipelines, providers, vocabulary and output.

## Primary tasks and evidence

Know at a glance that Sonora is ready (or live) and how to start; find and act on a past dictation; change one setting without wading through the rest. Code, local persisted data and Tauri IPC are the only authority for capabilities.

## Constraints

Preserve every existing workflow, pt-BR copy, keyboard access, local-only credentials. Navigation consolidates to four sections (Início, Histórico, Insights, Ajustes) without removing any function: file upload and Scratchpad live in Histórico; Atalhos, Recuperação and Diagnóstico live in Ajustes. User complaints to resolve: many buttons with no apparent function; information too crowded.

## Chosen direction

Estúdio no ar: Sonora as a quiet broadcast studio. Everything is off-air and neutral until you speak; the only lit thing is the tally lamp.

## Memorable moment

The tally lamp beside the wordmark lights red the moment the gadget goes live and amber while the take is processed, then goes dark again.

## Direction contract

THESIS: A studio that is silent until you speak. Refuses the card-mosaic dashboard and the toolbar of always-visible icon buttons; the screen shows content, and actions surface on hover, focus or state.

OWN-WORLD: One numbered neutral ramp (n0–n10, studio grey) is the only neutral token set and inverts for the dark studio at night. Colour is quarantined to state: tally red (live, destructive), standby amber (processing, warnings), cue green (done). Segoe UI Variable for every word; time, duration and keys in tabular Cascadia Mono like a broadcast clock. One hairline weight; no box inside a box.

STORY: The visitor sees immediately whether Sonora is ready or live and which key starts it, finds any past take in a log grouped by day, and changes one setting at a time in a settings column that reads top to bottom.

FIRST VIEWPORT: Sidebar 220px: wordmark with tally lamp, four text-led nav items, version at the foot. Content column max 760px: a 32px headline "Pronto para ditar" (becomes "No ar" / "Processando" with the lamp state), the toggle hotkey as large keycaps, one quiet line naming the next take's destination and mode with inline selects; then "Últimos ditados" as a log of five rows: time · text · duration. Nothing else.

FORM: Broadcast studio (on-air tally + studio log), position 7 of the ordered grounded list; seed key e6bb83ea. Raises: from the darkroom zone record, one numbered ramp as the only tonal tokens; from the lexicon, transcripts on a fixed 68ch measure and colour quarantined to state; from the ticket wallet, nothing disappears, it cancels (removed items stay recoverable, shown struck); from the Japanese high-density web, one hairline rule logic, never nested containers; from the character catalog, every log row keeps a fixed anatomy (time · text · duration · actions) in the same place.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance

## Unresolved decisions

None.
