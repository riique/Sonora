---
name: Sonora
description: A quiet studio for dictation — silent until you speak.
colors:
  studio-white: "#ffffff"
  studio-canvas: "#f8f8f6"
  studio-booth: "#f1f1ee"
  studio-fill: "#e9e9e5"
  studio-hairline: "#dfdfda"
  studio-hairline-strong: "#cacac4"
  studio-faint: "#a3a39d"
  studio-muted: "#67675f"
  studio-soft: "#4b4b47"
  studio-strong: "#2b2b28"
  studio-ink: "#171716"
  night-raised: "#232322"
  night-canvas: "#161615"
  night-booth: "#1b1b1a"
  night-fill: "#262625"
  night-hairline: "#31312f"
  night-hairline-strong: "#454543"
  night-faint: "#6a6a66"
  night-muted: "#a3a39d"
  night-soft: "#c4c4be"
  night-strong: "#e2e2dd"
  night-ink: "#f2f2ee"
  tally-red: "#c4322b"
  tally-red-lamp: "#e5483f"
  tally-red-wash: "#fbeeec"
  standby-amber: "#9a5c00"
  standby-amber-lamp: "#e09a2b"
  cue-green: "#276b41"
  cue-green-wash: "#e9f3ec"
  night-tally-red: "#ff6b61"
  night-standby-amber: "#e3a23b"
  night-cue-green: "#5cc283"
typography:
  headline:
    fontFamily: "Segoe UI Variable Display, Segoe UI Variable Text, Segoe UI, system-ui, sans-serif"
    fontSize: "26px"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "-0.02em"
  headline-ready:
    fontFamily: "Segoe UI Variable Display, Segoe UI Variable Text, Segoe UI, system-ui, sans-serif"
    fontSize: "32px"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "-0.02em"
  title:
    fontFamily: "Segoe UI Variable Display, Segoe UI Variable Text, Segoe UI, system-ui, sans-serif"
    fontSize: "19px"
    fontWeight: 600
    letterSpacing: "-0.015em"
  section:
    fontFamily: "Segoe UI Variable Text, Segoe UI, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 600
    lineHeight: 1.54
  body:
    fontFamily: "Segoe UI Variable Text, Segoe UI, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.54
  reading:
    fontFamily: "Segoe UI Variable Text, Segoe UI, system-ui, sans-serif"
    fontSize: "13.5px"
    fontWeight: 400
    lineHeight: 1.6
  description:
    fontFamily: "Segoe UI Variable Text, Segoe UI, system-ui, sans-serif"
    fontSize: "12.5px"
    fontWeight: 400
    lineHeight: 1.6
  meta:
    fontFamily: "Segoe UI Variable Text, Segoe UI, system-ui, sans-serif"
    fontSize: "11.5px"
    fontWeight: 400
  timecode:
    fontFamily: "Cascadia Mono, Cascadia Code, Consolas, monospace"
    fontSize: "11.5px"
    fontWeight: 400
    fontFeature: "tnum"
rounded:
  tag: "6px"
  menu-item: "7px"
  control: "8px"
  field: "9px"
  menu: "10px"
  surface: "12px"
  pill: "9999px"
spacing:
  "1": "4px"
  "2": "8px"
  "3": "12px"
  "4": "16px"
  "5": "20px"
  "6": "24px"
  "8": "32px"
  "10": "40px"
  "12": "48px"
  "14": "56px"
components:
  button-primary:
    backgroundColor: "{colors.studio-ink}"
    textColor: "{colors.studio-canvas}"
    typography: "{typography.body}"
    rounded: "{rounded.field}"
    padding: "0 16px"
    height: "36px"
  button-primary-hover:
    backgroundColor: "{colors.studio-strong}"
    textColor: "{colors.studio-canvas}"
  button-secondary:
    backgroundColor: "{colors.studio-white}"
    textColor: "{colors.studio-ink}"
    typography: "{typography.body}"
    rounded: "{rounded.field}"
    padding: "0 16px"
    height: "36px"
  button-secondary-hover:
    backgroundColor: "{colors.studio-fill}"
  button-ghost:
    backgroundColor: "transparent"
    textColor: "{colors.studio-soft}"
    typography: "{typography.body}"
    rounded: "{rounded.field}"
    padding: "0 16px"
    height: "36px"
  button-danger:
    backgroundColor: "transparent"
    textColor: "{colors.tally-red}"
    rounded: "{rounded.field}"
    height: "36px"
  button-small:
    rounded: "{rounded.control}"
    padding: "0 12px"
    height: "32px"
  field:
    backgroundColor: "{colors.studio-white}"
    textColor: "{colors.studio-ink}"
    typography: "{typography.body}"
    rounded: "{rounded.field}"
    padding: "0 12px"
    height: "36px"
  segmented:
    backgroundColor: "{colors.studio-fill}"
    textColor: "{colors.studio-muted}"
    rounded: "{rounded.field}"
    height: "34px"
  segmented-selected:
    backgroundColor: "{colors.studio-white}"
    textColor: "{colors.studio-ink}"
    rounded: "{rounded.menu-item}"
  nav-item:
    textColor: "{colors.studio-muted}"
    rounded: "{rounded.control}"
    padding: "0 12px"
    height: "36px"
  nav-item-active:
    backgroundColor: "{colors.studio-fill}"
    textColor: "{colors.studio-ink}"
  keycap:
    backgroundColor: "{colors.studio-white}"
    textColor: "{colors.studio-ink}"
    typography: "{typography.timecode}"
    rounded: "{rounded.tag}"
    height: "24px"
  keycap-large:
    rounded: "{rounded.menu}"
    height: "44px"
  tally-lamp:
    backgroundColor: "{colors.studio-hairline-strong}"
    rounded: "{rounded.pill}"
    size: "8px"
  tally-lamp-live:
    backgroundColor: "{colors.tally-red-lamp}"
  tally-lamp-standby:
    backgroundColor: "{colors.standby-amber-lamp}"
---

# Design System: Sonora

## Overview

**Creative North Star: "Estúdio no ar"**

Sonora is a quiet broadcast studio. Everything is off-air and neutral until you speak; the only thing that lights up is the tally lamp. The main window is where you review takes and tune the equipment, not a dashboard to admire: content sits in one ruled column, settings read top to bottom one row at a time, and every action that is not the point of the screen waits behind hover, focus, or an overflow menu.

The world lends the interface four things only: a single numbered grey ramp that inverts for the studio at night, a workhorse Windows sans for every word, a broadcast clock's tabular mono for time, duration and keys, and one signature move, the tally lamp. Navigation, controls and layout stay standard for a Windows productivity tool. The floating dictation pill (gadget) keeps its own black form over any application and shares only the state colours.

**Key Characteristics:**

- One column, hairline rules, no cards and never a box inside a box.
- Colour appears only when a real state needs it: live, standby, done, failure.
- Actions are revealed on hover and focus; a row at rest shows only its content.
- Every list of takes shares one anatomy: time over duration · text · actions.
- Light and dark are the same ramp, read in opposite directions; the theme follows Windows by default.

## Colors

A numbered studio-grey ramp carries every surface, line and word; three state colours are kept in quarantine.

### Primary

- **Studio Ink** (studio-ink / night-ink): primary text, primary buttons, the selected radio and the active toggle track. In dark it becomes the near-white end of the same ramp.

### Tertiary

- **Tally Red** (tally-red, lamp tally-red-lamp, wash tally-red-wash; night-tally-red): live recording, failures and destructive actions. The lamp glows; text uses the darker reading value.
- **Standby Amber** (standby-amber, lamp standby-amber-lamp; night-standby-amber): processing, fallbacks used and checks that need attention.
- **Cue Green** (cue-green, wash cue-green-wash; night-cue-green): saved, copied, configured, healthy.

### Neutral

- **Studio Canvas** (studio-canvas / night-canvas): the window ground behind all content.
- **Studio Booth** (studio-booth / night-booth): the sidebar plane, one step off the canvas.
- **Studio White** (studio-white / night-raised): fields, menus and raised controls.
- **Studio Fill** (studio-fill / night-fill): hover washes, the active nav item, segmented track, skeletons.
- **Hairline** (studio-hairline / night-hairline): every rule and border; the strong step marks hovered fields, keycap edges and the resting lamp.
- **Muted Graphite** (studio-muted / night-muted): descriptions, metadata and timecodes; the lowest step allowed for text (≥4.5:1 on canvas in both themes).
- **Faint** (studio-faint / night-faint): non-text only: icons at rest, separators like the keycap "+", list markers.

### Named Rules

**The Quarantine Rule.** Outside the grey ramp, colour only ever names a state that is true right now. It is never a brand accent, a section fill or decoration.

**The One Ramp Rule.** Neutrals come only from the numbered ramp (n0–n10). Dark mode is the same ramp inverted, never a separate palette.

## Typography

**Display Font:** Segoe UI Variable Display (with Segoe UI Variable Text, Segoe UI, system-ui)
**Body Font:** Segoe UI Variable Text (with Segoe UI, system-ui)
**Label/Mono Font:** Cascadia Mono (with Cascadia Code, Consolas)

**Character:** The Windows sans does all the talking so the app feels native; the broadcast clock's mono appears only where a value is time, a duration, a key or a technical identifier.

### Hierarchy

- **Headline** (600, 26px, 1.2, -0.02em): the page title of each section.
- **Headline Ready** (600, 32px): the Início state line, "Pronto para ditar" / "No ar" / "Processando o ditado", with the elapsed clock beside it in mono.
- **Title** (600, 19px, -0.015em): the current Ajustes subsection.
- **Section** (600, 13px): group headings; carry the hierarchy through weight, not size.
- **Reading** (400, 13.5px, 1.6): transcripts and notes, on a 68ch measure.
- **Body** (400, 13px, 1.54): rows, controls, sentences.
- **Description** (400, 12.5px, 1.6): the explanation under a setting, at most 60–64ch.
- **Meta** (400, 11.5px): counts, word totals, labels above a value.
- **Timecode** (400, 11.5px mono, tabular): time, duration, model IDs, paths, percentages.

### Named Rules

**The Broadcast Clock Rule.** Mono is for measurement only: time, duration, keys, IDs and paths. Never use it to make a label look technical.

**The No Eyebrow Rule.** A heading speaks for itself. No small label sits above a heading to introduce it.

## Layout

The shell is a 220px booth sidebar (64px icon rail under 900px) and a scrolling main area. Every page shares one left edge: the shell is up to 1040px wide with 48px side padding (32px under 1100px) and a 64px top margin under the transparent title bar. Início, Histórico and Insights read in a left-aligned 784px column; Ajustes uses the full width with a 168px text sub-nav and a 720px settings column, collapsing to a horizontal sub-nav under 1040px.

The rhythm is a 4px base, mostly 12, 16, 24, 40 and 48–56px. Sections are separated by 48px of space, not by containers. Settings rows are at least 64px tall with 40px between label and control. Lists use hairline dividers; Histórico groups takes by day under a ruled day heading and paginates only past 50.

## Elevation & Depth

Flat by default. Depth comes from the ramp step between canvas, booth and raised fields, plus 1px hairlines. Shadows exist only for things that float or behave physically.

### Shadow Vocabulary

- **Menu** (`0 12px 32px -12px rgb(20 20 18 / 0.22), 0 2px 6px -2px rgb(20 20 18 / 0.08)`; darker in night): open overflow menus, the save-status pill and dialogs.
- **Keycap** (`0 1.5px 0` in the strong hairline): the bottom edge of a physical key.
- **Tally glow** (a 3px ring plus a 12px bloom in the lamp colour): only while live or standby.

### Named Rules

**The Off-Air Rule.** A surface that neither floats nor reports live state gets no shadow and no glow.

## Shapes

Gently rounded and compact: controls 8–9px, menus 10px, the rare framed surface 12px. Full pills are kept for the tally lamp, the toggle, segmented thumbs, language chips and the gadget. Rules are 1px hairlines; containers usually use top and bottom rules only.

## Components

### Buttons

- **Shape:** 9px corners, 36px tall; small is 8px corners, 32px tall.
- **Primary:** Studio Ink fill with canvas text. One per view at most, reserved for the action the screen exists for.
- **Secondary:** raised fill with a hairline; hover adds the fill wash and strong hairline.
- **Ghost / Danger:** transparent until hover; danger text is Tally Red with a red wash on hover.
- **Disabled:** 40% opacity. A disabled primary drops to secondary so it never reads as a broken filled block.

### Overflow menu

One quiet "…" trigger replaces rows of icon buttons. It opens a 10px raised menu with the Menu shadow. Items are text with a 16px icon, arrow keys move between them, and Esc returns focus to the trigger. Destructive items sit last, after a hairline, in Tally Red.

### Log row (signature)

The studio log, used wherever takes are listed. The anatomy is a 52px column with time over duration in mono, then the text (one line in Início, up to three lines on the 68ch measure in Histórico), then a 104px actions slot. Actions (play, copy, …) appear on hover or focus. Play and copy stay visible while active. A failed take shows its one labelled recovery button.

### Inputs / Fields

- **Style:** raised fill, hairline border, 9px corners, 36px tall.
- **Focus:** a faint border plus a 3px ink ring at 8% opacity; the global focus-visible outline is 2px ink with a 2px offset.
- **Inline select:** inside a sentence, a select is just its underlined value with a small chevron, no box.

### Navigation

- **Sidebar:** text-led items, 36px tall. At rest muted with no fill; the active item gets the fill and ink. Under 900px only icons show, and labels stay in the accessibility tree.
- **Ajustes sub-nav:** text only. The active item is filled.
- **Segmented control:** a fill track with a raised thumb. Used for period, theme and Ditados/Notas.

### Tally lamp (signature)

An 8px lamp beside the wordmark (12px beside the Início headline). At rest it is an unlit hairline-grey bead with an inset shadow. Live, it glows Tally Red, and the sidebar names it "Gravando". Standby, it pulses amber. It follows the backend recording lifecycle and nothing else.

### Keycap

A raised fill and strong hairline with a 1.5px bottom edge, in mono. Small (24px) in rows and sentences, large (44px) for the Início hotkey.

## Do's and Don'ts

### Do:

- **Do** keep every neutral on the n0–n10 ramp and let dark mode invert it.
- **Do** reveal row and item actions on hover and focus, and name every icon-only control with an aria-label and a title.
- **Do** separate groups with 48px of space and hairlines before reaching for a container.
- **Do** use the log row anatomy (time over duration · text · actions) for any list of takes.
- **Do** keep descriptions to one or two short lines under the setting they explain.

### Don't:

- **Don't** use colour for anything but live, standby, done or failure.
- **Don't** put cards inside cards, or wrap a settings group in a box when hairlines already separate it.
- **Don't** show a row of always-visible icon buttons on list items.
- **Don't** add a small label above a heading, or a big-number metric trio as a page opener.
- **Don't** reintroduce orange, glow outside the tally lamp, or gradients.
