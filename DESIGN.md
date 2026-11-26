---
name: Cozy Rally
description: The game's own title-screen world, carried onto the web: cream cards, pastel lipped slabs, butter for the selected item.
colors:
  cream: "#fff6e6"
  cream-deep: "#f2e3cc"
  ink: "#3d3345"
  ink-soft: "#64576b"
  peach: "#fa9e85"
  mint: "#8cd1ad"
  sky: "#8cbff2"
  butter: "#fdd673"
  lavender: "#bfa8eb"
  berry: "#ed6b6b"
  rim-white: "#ffffff"
typography:
  display:
    fontFamily: "Fredoka, ui-rounded, Arial Rounded MT Bold, system-ui, sans-serif"
    fontSize: "128px (SVG logo, scaled to min(44rem, 100vw - 2.5rem))"
    fontWeight: 600
    letterSpacing: "-2px"
  headline:
    fontFamily: "Fredoka, ui-rounded, Arial Rounded MT Bold, system-ui, sans-serif"
    fontSize: "clamp(2rem, 1.3rem + 2.6vw, 3.4rem)"
    fontWeight: 600
    lineHeight: 1.1
    letterSpacing: "-0.01em"
  title:
    fontFamily: "Fredoka, ui-rounded, Arial Rounded MT Bold, system-ui, sans-serif"
    fontSize: "1.3rem"
    fontWeight: 600
    lineHeight: 1.1
  menu-label:
    fontFamily: "Fredoka, ui-rounded, Arial Rounded MT Bold, system-ui, sans-serif"
    fontSize: "clamp(1.15rem, 1.05rem + 0.35vw, 1.4rem)"
    fontWeight: 600
    lineHeight: 1.15
  pill-label:
    fontFamily: "Fredoka, ui-rounded, Arial Rounded MT Bold, system-ui, sans-serif"
    fontSize: "1rem"
    fontWeight: 600
  lede:
    fontFamily: "ui-rounded, SF Pro Rounded, system-ui, -apple-system, Segoe UI, Roboto, sans-serif"
    fontSize: "1.15rem"
    fontWeight: 400
    lineHeight: 1.6
  body:
    fontFamily: "ui-rounded, SF Pro Rounded, system-ui, -apple-system, Segoe UI, Roboto, sans-serif"
    fontSize: "1.0625rem"
    fontWeight: 400
    lineHeight: 1.6
  label:
    fontFamily: "ui-rounded, SF Pro Rounded, system-ui, -apple-system, Segoe UI, Roboto, sans-serif"
    fontSize: "0.8rem"
    fontWeight: 600
    letterSpacing: "0.01em"
rounded:
  code: "6px"
  badge: "12px"
  btn: "16px"
  card: "22px"
  pill: "999px"
spacing:
  stack: "0.9rem"
  menu-gap: "0.75rem"
  gutter: "1.25rem"
  band: "clamp(4rem, 8vw, 7.5rem)"
  container: "1180px"
components:
  menu-button:
    backgroundColor: "{colors.cream}"
    textColor: "{colors.ink}"
    typography: "{typography.menu-label}"
    rounded: "{rounded.btn}"
    padding: "0.85rem 2.75rem 0.95rem"
  menu-button-peach:
    backgroundColor: "{colors.peach}"
  menu-button-sky:
    backgroundColor: "{colors.sky}"
  menu-button-berry:
    backgroundColor: "{colors.berry}"
  menu-button-lavender:
    backgroundColor: "{colors.lavender}"
  menu-button-mint:
    backgroundColor: "{colors.mint}"
  menu-button-selected:
    backgroundColor: "{colors.butter}"
    textColor: "{colors.ink}"
  menu-button-small:
    rounded: "{rounded.btn}"
    padding: "0.65rem 1.6rem 0.75rem"
  pill:
    backgroundColor: "{colors.cream}"
    textColor: "{colors.ink}"
    typography: "{typography.pill-label}"
    rounded: "{rounded.pill}"
    padding: "0.3rem 1rem 0.35rem"
  chip:
    backgroundColor: "{colors.butter}"
    textColor: "{colors.ink}"
    rounded: "{rounded.pill}"
    padding: "0.1rem 0.55rem 0.15rem"
  card:
    backgroundColor: "{colors.cream}"
    textColor: "{colors.ink}"
    rounded: "{rounded.card}"
  card-white:
    backgroundColor: "{colors.rim-white}"
    rounded: "{rounded.card}"
    padding: "clamp(1.4rem, 3vw, 2.2rem)"
  note-badge:
    backgroundColor: "{colors.butter}"
    rounded: "{rounded.btn}"
    size: "4.2rem"
  postcard:
    backgroundColor: "{colors.cream-deep}"
    rounded: "{rounded.card}"
---

# Design System: Cozy Rally

## Overview

**Creative North Star: "The Title Screen, Left On"**

The website is not a store page that borrows the game's colours; it is the game's own front end, still running. Its world is inherited directly from the in-game UI in `crates/rally_game/src/ui.rs`: the same named constants (CREAM, CREAM_DEEP, INK, INK_SOFT, PEACH, MINT, SKY, BUTTER, LAVENDER, BERRY, SHADOW) converted to hex, the same Fredoka SemiBold lettering, the same lipped pastel menu slabs, and the same selection grammar (butter fill, white ring, play arrow). When the site and the game disagree, the game is the reference and the site follows.

The page reads as a sequence of full-width colour bands (cream, butter, sky, peach, cream, ink footer), each holding cream or white rounded objects that sit slightly lifted off the band. Density is relaxed: big rounded type, generous band padding, few elements per row. Real screenshots are the only imagery, framed as rounded postcards with a white rim. Everything that floats over a photo (tagline, hint, "Now showing", captions) is a cream pill.

Motion is slow and soft: a 14 s drift and 1.6 s cross-fade on the attract scenes, 0.25 s ease-out lifts on buttons, all on one long-tail ease-out curve, and all of it collapses under `prefers-reduced-motion`.

**Key Characteristics:**
- Palette and type are the game's, token for token; no web-only hues.
- Pastel slabs with a darker bottom lip, never flat rectangles.
- One selected item at a time, shown in butter with a white ring.
- Cream pills for any text laid over imagery.
- Soft plum-tinted drop shadows, never grey or black.

## Colors

A warm cream ground with a plum-ink line and a five-pastel accent set, plus butter reserved for "selected".

### Primary
- **Butter Yolk** (butter): the selection colour. Fills the selected menu button, the pace-note and lobby-step badges, the "new" chip, the skip link and text selection. It also carries one whole band (pace notes), where it reads as the game's focus colour spread wide.

### Secondary
- **Apricot Peach** (peach): the Windows download and the online band. The first accent in the logo.
- **Dusk Sky** (sky): the Android download and the garage band.
- **Berry Red** (berry): the "Race friends online" button only, at menu-label size.
- **Soft Lavender** (lavender): the "What's inside" button and the "circuit" chip on circuit cars.
- **Spearmint** (mint): the "How to install" button; a logo letter colour.

### Neutral
- **Warm Cream** (cream): page ground, cards, pills, the default menu button, the footer's text.
- **Toasted Cream** (cream-deep): postcard backing while images load; scrollbar track.
- **Plum Ink** (ink): all primary text, the logo's outer outline, focus outlines, the play arrow, pace-note icon strokes, the footer ground.
- **Faded Plum** (ink-soft): secondary text on cream (ledes, card body, captions' labels, list markers), scrollbar thumb.
- **Rim White** (rim-white): the 4px ring around selected buttons and postcards, the logo's inner rim, and the install cards.

### Named Rules
**The Game Owns The Palette Rule.** Every colour is a `ui.rs` constant. A new surface picks from these eleven; it does not mix a new hue. Translucent cream (`rgb(255 246 230 / 0.88-0.94)`) and translucent ink (`rgb(61 51 69 / 0.06-0.16)`) are tints of existing tokens, not new colours.

**The Butter Means Selected Rule.** On interactive elements, butter is the selected/hover state. Never give a resting button a butter fill; it would read as already chosen.

**The Ink On Pastel Rule.** Text on every pastel is ink, never white. Berry is the darkest pastel (about 3.9:1 against ink), so it carries only large display labels, not body copy.

## Typography

**Display Font:** Fredoka SemiBold 600 (self-hosted `docs/fonts/Fredoka-SemiBold.ttf`, fallback ui-rounded, Arial Rounded MT Bold)
**Body Font:** the platform rounded/system sans (ui-rounded, SF Pro Rounded, system-ui, Segoe UI, Roboto)

**Character:** Fredoka does all the talking that the game would do: logo, headings, button labels, pills, badges, list markers. The body face is a quiet system sans so long copy stays readable; it is a provisional stand-in (only the SemiBold Fredoka cut ships in the repo), not a chosen pairing.

### Hierarchy
- **Display** (600, 128px in the SVG logo, scaled to the column, -2px tracking): the multicolour outlined logo only.
- **Headline** (600, clamp(2rem, 1.3rem + 2.6vw, 3.4rem), 1.1, -0.01em): one band heading per section, balanced wrap.
- **Title** (600, 1.3rem, 1.1; 1.2rem in lists, 1.6rem on install cards): car names, pace-note calls, lobby steps.
- **Menu label** (600, clamp(1.15rem, 1.05rem + 0.35vw, 1.4rem), 1.15): menu slab labels.
- **Pill label** (600, 0.95-1.2rem): tagline, hint, "Now showing", postcard captions.
- **Lede** (400, 1.15rem, 1.6, max 62ch): the one paragraph under each headline.
- **Body** (400, 1.0625rem, 1.6): card and list copy, 0.9-1rem in dense lists.
- **Label** (600 body face, 0.8rem, 0.01em): the meta line under download labels (format, size).

### Named Rules
**The Fredoka Speaks Rule.** Anything that names, labels or invites a press is set in Fredoka 600. Sentences of explanation are set in the body face. There is only one Fredoka weight; hierarchy comes from size, never weight.

## Layout

A single column of full-bleed colour bands, each with vertical padding clamp(4rem, 8vw, 7.5rem) and a centred container of min(1180px, 100% - 2.5rem). The title screen is the exception: a 100svh stage with the menu column (max 33rem) on the left and the photo filling the rest, with a cream veil fading from the menu side.

Inside bands, content sits in simple grids: a 3-column postcard mosaic (one tile spanning 2x2), a 0.8fr/1.2fr split with a sticky intro beside a list (pace notes), a 3-column card grid (garage), two equal columns (online, install). Gaps are fluid clamps around 0.75-1.4rem for tiles and 2-5rem between columns; stacks inside the menu use 0.75-0.9rem.

At 960px the split layouts collapse to one column and the garage goes to two. At 720px the title stage becomes a 40svh photo strip on top with the menu on cream below, the menu becomes a 2-column grid with the platform downloads spanning both columns, and keyboard hints are hidden.

## Elevation & Depth

Depth is tactile and toy-like: every raised object combines a soft plum-tinted drop shadow with an inner bottom lip (an inset shadow along the lower edge) that makes it look like a pressed slab. Selection lifts further and adds a white ring. Nothing uses grey or black shadows, and nothing uses hard offset shadows.

### Shadow Vocabulary
- **Rest** (`box-shadow: 0 6px 14px rgb(38 26 51 / 0.22)`): pills, cards, postcards, menu buttons. The game's SHADOW constant.
- **Lift** (`box-shadow: 0 10px 22px rgb(38 26 51 / 0.28)`): selected/hovered buttons and the pace-notes panel.
- **Lip** (`box-shadow: inset 0 -6px 0 rgb(61 51 69 / 0.16)`): menu buttons; lighter 0.06-0.14 alpha and 4-5px on cards and badges.
- **Rim** (`box-shadow: 0 0 0 4px #fff`): selected buttons and every postcard.

### Named Rules
**The Lipped Slab Rule.** Buttons and badges always carry the bottom lip. Pressing a button shrinks the lip (6px to 2px) and moves it down 3px, so the lip is the press feedback.

**The Plum Shadow Rule.** Shadows are tinted rgb(38 26 51); a neutral black shadow breaks the world.

## Shapes

Everything is rounded, in three main steps: 16px for buttons and pace-note badges, 22px for cards, panels and postcards, fully round (999px) for pills and chips. Smaller pieces (lobby step numbers, the skip link, phone-size badges) use 12px; inline code uses 6px. There are no hairline borders: edges come from fill contrast, the lip and the white rim. The only line is the 2px dashed ink divider between pace-note calls, which reads like a co-driver's notebook. The logo is lettered three times over (ink outline, white rim, coloured fill) with round joins.

## Components

### Menu Buttons
Pastel slabs with a lip, stacked like the game's menu; the downloads are menu items.
- **Shape:** gently rounded slab (16px).
- **Primary:** one pastel per destination (peach Windows, sky Android, berry online, lavender inside, mint install, cream source), ink label centred, optional meta line below in the body face.
- **Selected:** exactly one item selected at a time: butter fill, 4px white ring, Lift shadow, scale(1.035), and an ink play-arrow sliding in from the left. Hover, focus and arrow keys move the selection; the visitor's platform download is selected by default.
- **Focus:** 3px ink outline at 6px offset, in addition to the selected look.
- **Small:** 0.65rem 1.6rem padding, 1.05rem label, no arrow, no scale; hover gives butter, ring and lift. Used for secondary downloads (server, install cards).

### Pills
Cream (88-94% opaque) fully round labels with the Rest shadow for any text over a photo or band: tagline, hint bar, "Now showing" (whose name fades and drops 0.35rem on change), postcard captions.

### Chips
Tiny fully round labels (0.75rem): butter "new", lavender "circuit" after the car name.

### Cards / Containers
- **Corner Style:** 22px.
- **Background:** cream on coloured bands (garage cars, lobby steps, pace-notes panel); white on the cream install band.
- **Shadow Strategy:** Rest plus a light 5px lip; the pace-notes panel uses Lift.
- **Border:** none.
- **Internal Padding:** 0.9-1.4rem in list cards, clamp(1.4rem, 3vw, 2.2rem) in install cards.

### Postcards
Real screenshots in 22px frames with a 4px white rim, 16:10, a cream pill caption bottom-left, and a slow 1.04 zoom on hover.

### Badges
Butter rounded squares with a lip holding either a stroked ink SVG pace-note icon (4.2rem, 4.5 stroke, round caps) or a lobby step number in Fredoka.

### Navigation
The title menu is the navigation; there is no separate header bar. The footer is a plain ink band with cream text and cream-tinted underlines.

## Do's and Don'ts

### Do:
- **Do** take every colour from the `ui.rs` set and set ink text on every pastel.
- **Do** give every button and badge the bottom lip and the plum Rest shadow (`0 6px 14px rgb(38 26 51 / 0.22)`).
- **Do** show selection the game's way: butter, 4px white ring, Lift shadow, play arrow; one selected item per menu.
- **Do** float text over imagery only inside a cream pill.
- **Do** use real in-game screenshots, framed as rimmed 22px postcards.
- **Do** keep all motion on `cubic-bezier(0.16, 1, 0.3, 1)` and disable drift and cross-fades under reduced motion.

### Don't:
- **Don't** introduce colours or shadow tints that the game UI doesn't have; gradients appear only as the cream or ink veil that fades a photo into the menu.
- **Don't** use butter as a resting fill on an interactive element.
- **Don't** set white text on pastels or body copy on berry.
- **Don't** add hairline borders or hard offset shadows; edges come from fills, lips and the white rim.
- **Don't** use more than one Fredoka weight or set long paragraphs in Fredoka.
