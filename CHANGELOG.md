# Changelog

All notable changes to SMB3-RS are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
New work accumulates under **[Unreleased]** and is moved into a new versioned
section when a release is cut — a merge to `main`, which bumps the version and
deploys.

## [Unreleased]

## [1.3.0] - 2026-09-05

### Added

- **King quotes** (new option, on by default). The kings' rescue dialogue is
  randomized, as it always has been; turn this off and they say what they say in
  the original game instead. Cosmetic, so it is not carried in the flag key —
  and the seed is unaffected either way, so two players on the same key get the
  same game whichever way each of them sets it.

- **Deja Vu** (new option, off by default; MaCobra52's idea). Lets the same
  level show up on more than one tile. *Double* puts a second copy of every
  level in the deck, so some appear twice and others sit the seed out. *Wild*
  deals with replacement — a level can turn up any number of times, or never.
  Levels that hand out a one-off item (the chest levels and the World 8 hand
  rooms) still appear exactly once in both modes.

- **Bro Battle Timer** (new option, off by default). Walking into a Hammer,
  Boomerang, Heavy or Fire Bro starts the clock at 10 instead of the arena's
  usual 200 — and with the clock fix in this release, ten really is ten
  seconds. Miss it and the fight takes the life instead. Only encounters
  entered through a bro sprite are affected; every other level, including
  World 8's full-length Hammer Bro entry, keeps its own time.

- **Limit Hazards** (new option: Off / Some / All, off by default). Stops enemy
  swaps from dropping a hazard into a level that wasn't built for one — nippers,
  Ptooies, thwomps, Hot Foots and Bros. Hazards Nintendo placed are always kept,
  and a slot that held one can still shuffle within its own kind, so this only
  ever removes surprises the randomizer added. **Some** allows the occasional
  one so you still learn to handle them; **All** allows none. Measured over 250
  seeds of the Max Chaos preset, the unfiltered game gains ~140 hazards a seed
  with as many as 25 in a single stretch of level; Some cuts that to ~52 and
  never more than one.

  Only in-level enemies are affected. Hammer Bro mini-battles keep their own
  curated pools, and Wild Injections are unchanged.

- **Friendlier Levels** (new option, off by default). Keeps the roughest levels
  out of the shuffle — 2-3, 5-3, 6-6, 7-5, 7-8 and 8-1. Their slots go to beta
  stages when those are on, and otherwise to a second visit to a level already
  in the seed, so the map is exactly as full as it always was.

  With lobby shuffle on, the held-back levels sit that out too. Blocking a level
  removes its *front door*, and the lobby shuffle moves *interiors*
  independently — so a blocked level left in that pool would have its interior
  donated to a door you can still open, while whatever was donated to the
  blocked door became unreachable, quietly costing you an unrelated level.

  It also makes two fortresses — 7F2, then 8F1 — optional rather than absent.
  They stay on the map and stay beatable, but land behind a lock the world can
  be finished without, so you can walk past them. Forts can't be held out of the
  pool the way levels can (every fort needs a lock, and the full roster is an
  invariant), and there has to be a spare bypassable lock going: 1-F always
  claims the first one, since its secret exit can leave a lock shut forever.
  Measured over 300 seeds, 99% have room for both; in the rest 8F1 stays
  required.

- **Shuffle Big ? Rooms** (new option, off by default). Every level with a Big ?
  pipe draws from a pool of 19 rooms instead of always opening its own — the 11
  vanilla rooms plus 8 from "Unused Level 5", a complete set of eight bonus
  rooms left in the ROM and reachable by nothing. Vanilla's 15 rooms are only
  10 distinct layouts, so this roughly doubles what you can walk into. 7-F1's
  block is still always a flight suit, whichever room it draws, because the
  level needs one.

  Flag keys shared during the beta, when this ran unconditionally, now decode
  with it **off** — tick the box to get those seeds back.

- **Three new king rescue quotes** join the pool the kings draw from.

- The version manifest now publishes the **flag-key format version**, which lets
  the racetime seed bot reject a stale key *before* it posts the seed link.
  Previously it could only check a key's shape, and keys older than the current
  format carry no checksum — so a leftover key from an older randomizer sailed
  through, the bot posted the link, and every racer's browser then refused the
  key and fell back to whatever settings they happened to have. Racers in the
  same race could end up playing different rulesets with nothing on screen to
  say so. The bot now answers with which version the key is from.

### Changed

- **Fortresses no longer sit where you cannot walk around them.** A fortress in
  the middle of the only road is a wasted lock — you play it because it is in
  the way, and the lock it opens was never a decision. Fortresses now take a
  spot you could have walked past, so going to find one is a choice you make.
  Measured across 1000 seeds, unavoidable fortresses went from 22% to under
  0.5%, and worlds came out with slightly more routes rather than fewer.

- **The level clock now counts real seconds.** Vanilla steps the timer every 41
  frames, so a unit was 0.68 s and the clock ran about 47% fast — a "300" level
  was really about 3:25, not five minutes. The divider is now 60 frames, giving
  a measured 0.998 s per unit, on every seed.

  The reason is less about the clock than about what it lets us build: a timer
  that reads seconds is a unit other options can use. **Bro Battle Timer**
  says ten seconds and means ten seconds, and anything timed added later gets
  that for free.

  Worth being straight that the fast clock was a *choice*, not a bug — the
  stored divider is 40, exactly 2/3 s at 60 Hz, and the PAL release uses the
  same value, so it was never scaled to a refresh rate. Levels are therefore
  more generous in real terms than Nintendo shipped, though running the clock
  out was already a rare way to lose a seed.

- **Beginner Friendly** preset now uses Limit Hazards (All) and Friendlier
  Levels, and has its Ghosts class turned back on. The class was previously
  disabled outright just to stop Boo → Hot Foot; blocking that directly brings
  Boo ↔ Dry Bones variety back.

### Fixed

- Levels, fortresses and locks no longer land on the map cell whose completion
  bit is already taken by scenery. Rows 7 and 8 of a world map share one bit
  per column, and the game checks row 7 first — so World 2's oasis, which sits
  on row 7 and counts as scenery rather than a level, quietly swallowed the bit
  for anything placed directly below it. A level there could never show beaten,
  a fortress never crumbled, and a lock grew back every time you re-entered
  the world.

- Wandering Hammer Bros are spread further apart on the world map. They were
  being placed as close as two tiles, which is exactly one of their marching
  steps — a bro could land on top of another, and the game makes both of them
  march again, so a crowded world could keep marching for a very long time
  before handing you back your turn.

- One of the new Big [?] Block bonus rooms could take your powerup and give
  nothing back. Its block sits across the room from where the pipe drops you,
  reachable only by steering mid-fall; miss that and you land in a pit with
  nothing to climb and no way to try again — the room is spent. Two coins in
  the shaft are music blocks now, so the floor is a setback instead of a dead
  end. Only affects seeds with Big [?] room shuffle turned on.

- A randomized Hammer Bro encounter can no longer trap you in the floor. The
  flying red Paratroopa ignores level geometry and sinks about seven tiles
  below where it starts, so on a Bro's ground row it spent half its cycle
  inside the floor — and hitting it as it climbed back out could leave you
  stuck in the ground, in a room whose exit only appears once it is cleared.
  It now starts high enough that the bottom of its dive lands *on* the floor.

- Travelling through a pipe no longer kills you when the screen scrolls at the
  same time. The game's "crushed against the left edge" check only looks at how
  far left Mario's sprite has been pushed, and a pipe transition that squished
  the view could push him far enough to trip it — a death with nothing on
  screen to cause it. Pipe travel is now exempt, the same way vertical levels
  and a finished level already were. (MaCobra52's "Pipe Screen Squish Fix".)


## [1.2.1] - 2026-08-26

### Fixed

- The **Super Princess Peach** visual patch drew a band of garbage tiles across
  the title screen. Peach's title logo needs room in the ROM bank the title
  screen runs from, and took it from the same stretch of empty space the B-to-
  mute toggle was written to; the randomizer applies the visual patch first, so
  it landed on top of the logo data. The toggle moved to the far end of that
  bank, and a test now checks every bundled visual patch against the whole
  free-space registry.

- With autoscroll removal on, the **two Big Berthas in 4-1's underwater room
  and the two para-troopas in 5-4** were left vanilla in every seed. Removing
  an autoscroller writes a stream terminator into the enemy data, and the range
  that tells the randomizer where to pick the parse back up was one byte long,
  so it read that whole stretch out of step and skipped past both levels'
  enemies. Nothing was ever written to a wrong byte; those four simply never
  randomized. (#181)

## [1.2.0] - 2026-08-22

### Added

- **Lakitu Stays Down** (off by default). Beat a Lakitu and it stays down,
  instead of drifting back in from behind you a few seconds later. Vanilla
  never lets one die, and while it's alive it also never despawns — so it
  permanently occupies one of the five slots the game has for enemies, and
  every Spiny Egg it throws takes another. That starves the rest of the level,
  and can leave a pick-up-able ice block refusing to lift at all.

- **Press B on the title screen to mute the menu music**, and B again to bring
  it back. Handy when you are generating and verifying seeds back to back, or
  streaming with the title screen up. Start still begins the game as normal,
  and the world map queues its own music either way.

- The web app now shows the seed's **title hash** under the Generate button as
  soon as a seed is entered — the same five icons the title screen will show,
  drawn from your own ROM's graphics (and from the selected visual patch, if
  one is chosen). Racers can compare icons before generating instead of after
  booting. Hidden while the seed box is empty, since a blank seed is rolled at
  generate time, and while ROM validation is off, since the hash isn't applied
  then.

- A "The End" sign-off in the web app's footer, drawn from the ending
  sequence's own graphics.

### Changed

- **World 8's bridges to Bowser's castle now vary.** How many of the five
  spans are out is rolled per seed — usually one or two, sometimes three,
  occasionally none at all, and about one seed in five thousand takes out
  four. Which spans go is drawn fresh each time. Previously it was almost
  always exactly one, and almost always the same one: the third span won the
  builder's ranking by a single tile every seed, and once it was taken the
  others had nothing left to gate.

- **World length varies more between worlds.** Every world used to guarantee
  the same minimum amount of play before its goal, which made the cheapest
  route recognisable once you knew the number to look for. Each world now gets
  its own minimum — a level's worth below, at, or above the old one — rolled
  per seed and different every time. Some worlds are dealt no variation at all.
  The overall amount of play across the eight worlds is unchanged.

- The web app's option icons are now decoded from your own ROM's graphics
  instead of bundled sprite sheets, so a selected visual patch re-skins them
  too. They appear once a ROM is loaded. Several options that never had an
  icon now have one — Boom-Boom Stomps, Cannons, Ghosts, Shell, Rotodiscs,
  Faster Frog, Swap Start / Airship, Shuffle Spade Games and Remove N-Cards.

- The 8-Tank treasure room now draws from the same enemy pool as the other
  bro-fight rooms, so its enemy follows Hammer Bro encounter randomization
  along with the rest.

### Fixed

- **Treasure-box rooms could be made unwinnable.** A room whose exit is a
  treasure box only opens once you have cleared it, so anything in it that
  cannot be killed traps you there. Two ways that happened:

  A Hammer Bro in such a room is not really an enemy — the game treats it as
  "whichever bro's map sprite you walked in through", and in a room you did not
  reach through a bro sprite it turns into something unkillable. This is what
  players hit in the Coin Ship's reward fight, and it could also reach the
  8-Tank and White Toad House reward rooms. Hammer Bros are now kept out of
  every such room.

  Dry Bones could also appear there. Stomping one works but it gets back up, so
  in a room you have to clear it is a dead end. It is now only ever placed
  alongside a Koopa shell, which does kill it for good.

  Both are unchanged everywhere else in the game.

- **The title hash no longer changes colour when you use a visual patch.** It
  used to encode part of itself in the icons' colour, drawn from Mario's
  palette — which every character re-skin rewrites, so the same seed showed red
  on a vanilla ROM, green under Luigi and cyan under Toad. Two racers on
  different skins saw different colours for the identical game, and there was no
  way to tell that apart from a genuinely different seed. The colour is now
  fixed and means nothing; the icon set grew from 15 to 20 instead, which more
  than replaces what the colour was worth (3,200,000 combinations, up from
  1,518,750). **Your title hash icons will differ from 1.1.0's for the same
  seed**, as they do on any version change.

- Option icons are no longer squashed. Every sprite was being forced into a
  24x24 box, which scaled 16x16 art by 1.5x and distorted the taller sprites;
  pixel art is now only ever scaled by a whole number.

## [1.1.0] - 2026-08-08

### Added

- **Random Boom-Boom Stomps** now has a toggle in the web app, under Bosses. It
  was already part of the randomizer and already carried in the flag key, but
  with no control on the page the web app always ran it on — and a shared flag
  key that turned it off was silently ignored. The setting is on by default, so
  nothing changes unless you turn it off.

- The beta stage β4 now joins the Antechamber Shuffle pool when beta stages
  are enabled. Like the vanilla antechamber levels, its 8-screen interior can
  be swapped behind another level's entry pipe (and vice versa). With beta
  stages off, output is unchanged. Note: turning on both beta stages and
  antechamber shuffle now produces a different result for a given seed than
  before (the shuffle covers one more level).

- The Visual Patch pills in the web app now show a preview sprite of what each
  patch gives you — the same standing pose rendered from each patch's own
  graphics, so you can see Luigi, Peach, Toad, Dr. Mario or Baldman before
  generating. The "+ Viruses" pill shows one of its viruses instead, since its
  player graphics are the same as plain Dr. Mario.

- Modern Power-Ups option (MaCobra52's "Easy Power-up System"): power-ups work
  like the newer Mario games — Small Mario grabbing a Fire Flower or suit gets
  its power without turning Big first. Off by default; on in the Beginner
  Friendly preset.

- Poison Mushrooms option (after MaCobra52's "All 1UPs are Poison Mushrooms"):
  every 1-Up block becomes a coin flip. Each one independently hands out either
  a real 1-Up or an upside-down poison mushroom that hurts you, decided by the
  seed — so a run keeps some real 1-Ups mixed in with the traps, and you can't
  tell which a block holds until you hit it. Off by default; on in the Max
  Chaos and Challenging presets.

- Beta site is now visually distinct from the main site: the `/beta/` deploy
  shows a hazard-striped "BETA BUILD" banner, a violet frame, and a BETA badge
  in the header so it can't be confused with the stable release page.

- Canoe "call the boat" rescue: stand on any dock and press A to summon the
  canoe to the water beside you, then board as usual. Prevents canoe softlocks
  where the boat was left out of reach, in both 1- and 2-player games.
- Two-player "warp to partner" escape hatch: on the overworld map, the active
  player can press Start+Select to jump to the other player's tile. This
  prevents softlocks where one player moves a shared map object (such as the
  `8s are Wild` canoe) out of the other's reach. No effect in 1-player games.

### Changed

- The presets have been refreshed to take in this release's new options, and
  there's a new **League Season 7** preset.

- **Flag keys have been rebuilt, and keys from earlier versions no longer
  work.** This is the last time they break: the new format has room to grow, so
  from here on adding an option leaves existing keys valid — the new option is
  simply off in a key made before it existed. Two other things come with it:

  - **A mistyped key is now caught.** Previously a single wrong character
    produced a valid key for *different* settings about nine times out of ten,
    with nothing on screen to say so. A key now carries a check byte, so a typo
    or a truncated paste is rejected instead of quietly changing the ruleset —
    which matters most in a race, where everyone is pasting the same key.
  - **Keys got a little shorter** (26 characters instead of 27 for the default
    set), and only carry as much as the settings need.

- "Wild Injections" now lets you pick which chasers get dropped into levels,
  and adds a third one. The option is four pills — Off, Sun, Lakitu, Bass —
  where the three chasers toggle independently and Off clears them, so you can
  ask for any combination. Picking fewer doesn't make them rarer; every
  combination drops about the same number per seed.

  **Boss Bass is new to the pool**: a leaping Big Bertha that follows you
  through a level that never had one, dry ground included. It was meant to be
  available when the option first shipped, but with water enemies shuffled it
  was quietly turned back into an ordinary fish before it ever reached the ROM.

  Saved web settings carry over (an old "on" becomes Sun + Lakitu, the pool as
  it stood then), and the CLI flag takes a set now: `--wild-injections sun,bass`,
  or `all`, or `off`.

- World 1 has a new rock sitting between the middle of the map and the
  bottom-right. With "More hammer rocks" on you can break it, opening a real
  second way around a world that otherwise runs as one long lap. With the
  option off the rock is solid — and it looks exactly the same either way, so
  you have to swing a hammer at it to find out.

- **The overworld builder was rebuilt around route choice.** Maps are no
  longer rolled and rerolled until one looks acceptable. Each world is laid
  out plainly first — bridge the cut-off parts with pipes, then place levels,
  fortresses and locks — then measured for how many roughly-equal routes
  reach the goal, then shaped by targeted moves: a fortress re-prices a
  shortcut, a lock forces that fortress's cost onto it (the classic "beat the
  fort or take the long way round"), a pipe opens an alternative. A world
  that still won't fork redeals its whole pipe web and tries again.

  What that means to play: **worlds with only one reasonable way through are
  now about 7% of seeds**, where before the rebuild the same measure sat in
  the twenties, and a typical world offers roughly two and a half distinct
  routes. Worlds whose terrain genuinely can't fork stay honestly linear —
  World 7 is still the most stubborn at ~20%, while World 6 now forks
  essentially always.

  Structures the shaping can build, which you may notice by name:

  - a **gated shortcut** — a pipe whose approach is locked behind a fortress,
    so taking it means beating that fort first, rather than a free skip;
  - a **loop** wired between a world's islands, so it has two arms tied at
    similar cost instead of one spine — the shape vanilla World 7 ships;
  - **rock trades** — smashing a hammer rock now counts as a genuinely
    different route from walking around it, not a detour of the same one.

  Two guarantees hold everywhere: the cheapest way through a world always
  costs at least about five levels' worth of effort, and every world still
  ships its full vanilla pipe count. Deterministic as always — the same seed
  still produces the same maps.

- The World 5 spiral-castle pipe no longer always draws the castle on the far
  endpoint — the castle and the pipe mouth are now coin-flipped between the
  pair's two cells, so the castle can appear on either side.

### Fixed

- A flag key the app can't read is now refused out loud instead of being
  quietly ignored. Previously the key stayed in the box, the options kept
  whatever they already were, and Generate stayed clickable — so a key from a
  different version of the randomizer looked accepted while your own leftover
  settings were used instead. That mattered most in races, where everyone
  pastes the same key and each person could end up on different settings with
  nothing on screen to say so. Now the key is marked as rejected, Generate is
  held until it's sorted out, and the message says whether the key is from an
  older version, from a newer one, or just isn't valid.

- Landing on an enemy that is jumping up at you now counts as a stomp instead
  of hurting you. Vanilla decides "stomp or damage" once per frame, so an enemy
  rising into you could close the gap faster than the check could resolve and
  the hit landed on you — most visibly with hopping Cheep Cheeps and with
  Koopalings at their higher jump speeds. The stompable range now cancels out
  the enemy's own upward speed, so the verdict no longer depends on how fast it
  was moving. Collisions vanilla already judged correctly are unaffected.

- Breaking a fortress lock in World 8's dark area no longer lights up the tile.
  The lock-break effect used to paint the replacement tile over the darkness,
  leaving it permanently visible. Now the poof plays where the lock was, and a
  lock out in the dark stays hidden until your light reaches it, while a lock
  already inside your light simply disappears as it should. Either way the lock
  is gone for good. As part of the same fix, beating a fortress whose lock you
  had already smashed with the hammer no longer redraws that tile — in the dark
  area that redraw gave away which lock belonged to which fortress, which is
  meant to be the risk you take when you swing the hammer blind.
- Map nodes no longer show up in the wrong scenery. An empty node sitting in
  World 5's sky region could be drawn as a green land tile, and nodes on the
  island strips in Worlds 3, 4 and 7 could get land tiles instead of sand —
  most visibly a green square floating in the middle of the clouds. Roughly
  3-4 tiles per seed. Nothing moved; only the artwork of those tiles changed.
- The beta site's "Share URL" button now shares a beta link. It was building a
  `/v/<version>/` link, but only main releases are archived there, so the link
  either 404'd or opened a different build with the same version number.
- Levels no longer pile up back-to-back in one corner of the map: level
  placement (and the shaping moves that relocate levels) now avoid putting
  a level next to another whenever the map allows. Worlds with 4+ levels
  chained in a row drop from ~1 in 6 to ~1 in 50, and route choice
  improved as a side effect (spread levels fork more).
- The "call the boat" canoe summon (press A on a dock) could drop the canoe on
  a land tile *inside* an island instead of on the water beside the dock,
  leaving it unboardable. Its tile check read one map row too low, so a path
  tile below the water could be mistaken for water. It now reads the correct
  cell (World 3's middle island dock was the visible case).
- The eight Dry Bones and the Roto-Disc in World 4's second fortress are now
  hazard-protected, so they can't be swapped for a Thwomp/Ptooie/nipper-style
  hazard sitting on the walking path.

## [1.0.7] - 2026-07-31

### Fixed

- Lobby (antechamber) shuffle could drop you into a void when you entered a
  Big ? Block bonus room inside 5-2 or 6-9, in two ways, both fixed:
  - **5-2's bonus room landed you in a garbage spot.** 5-2's bonus room reads
    its arrival position from the same pipe the shuffle was rewriting to relink
    the lobby — so it flung you into the room at a corrupted position. The
    shuffle now leaves that pipe alone.
  - **The bonus room itself was the wrong (void) one.** The fix that keeps these
    rooms working when a level moves worlds keyed on the map tile you entered —
    which, under lobby shuffle, is a *different* level's tile. It now keys on the
    room you're actually standing in, so 5-2 and 6-9 open their own bonus rooms
    even when reached through another level's lobby.

## [1.0.6] - 2026-07-27

### Fixed

- 7-1: the two Green Troopas near the start now always stay in the shell pool,
  so the enemy shuffle can no longer replace them with something unsafe there.

## [1.0.5] - 2026-07-25

### Changed

- The main site's tab title is now "SMB3 Randomizer" (previously carried a
  "(beta)" tag). The `/beta/` deploy is now visually distinct from the main
  site: it shows a hazard-striped "BETA BUILD" banner, a violet frame, and a
  BETA badge in the header, keyed on the URL path so it can't be confused with
  the stable release page.

## [1.0.4] - 2026-07-24

### Removed

- The "Pipe Shuffle" option (web checkbox and `--no-shuffle-pipes` CLI flag).
  It has been a no-op since the overworld builder took over pipe placement —
  pipes are always placed by the builder. Flag keys bump to v26; old keys
  encoding the dead bit are no longer accepted.

## [1.0.3] - 2026-07-21

### Fixed

- Archived version pages (`.../smb3-rs/v/<version>/`) were shipping without their
  WASM bundle, so they loaded a blank shell with no options. The snapshot step
  now force-includes `pkg/`, which `wasm-pack`'s generated `.gitignore` had been
  causing `git add` to skip.

## [1.0.2] - 2026-07-20

### Added

- Every version of the web app is now archived at a permanent URL
  (`.../smb3-rs/v/<version>/`). The site root keeps serving the latest build;
  each merged version is also frozen at its own path so it never changes. The
  "Share URL" button now points at the exact version that generated the seed, so
  a shared link keeps producing the same seed even after newer versions ship. A
  version picker in the footer lets players open any older build.

## [1.0.1] - 2026-07-20

### Changed

- The 7-Fortress 1 ? block that gates the Tanooki area now randomizes 50/50
  between a Fire Flower and a Super Leaf instead of always being a Fire Flower.
  It can never roll a star, so small Mario always gets a power-up that lets him
  break the bricks to reach the area.

## [1.0.0] - 2026-07-19

### Changed

- The title-screen seed-verification icons now depend on the randomizer version
  in addition to the seed and options. Two builds with different randomization
  logic no longer show identical icons for the same seed. (CI now requires a
  version bump on every merge to `main`, so each release is a distinct version.)

## [0.12.9] - 2026-07-18

### Fixed

- The title screen no longer rolls the attract-mode demo. Sitting on the 1P/2P
  menu now holds indefinitely instead of timing out into the recorded demo
  playback.

## [0.12.8] - 2026-07-18

### Fixed

- The final Big Green Troopa in 4-1 is now covered by the level's hazard
  protection, like the Big Red Troopas earlier in the stage. Each troopa sits on
  a small platform Mario must land on to progress, so a hazard enemy there could
  force an unavoidable hit.

## [0.12.7] - 2026-07-17

### Fixed

- Randomized enemies no longer place a Dry Bones in the Coin Ship reward fight.
  That room is enclosed and never scrolls, so a Dry Bones — which revives after
  every stomp and has nowhere to wander off — could never be cleared.

## [0.12.6] - 2026-07-17

### Changed

- Overworld shortcut pipes now vary how much they skip: each pipe rolls a random
  cap on how many forced levels it may bypass (usually 1–2, occasionally more)
  instead of always grabbing the largest possible skip. Big skips still happen,
  just less often — so a single pipe no longer routinely trivializes a short
  world like 2 or 6, while the overall maps stay less linear.

## [0.12.5] - 2026-07-17

### Changed

- The ending credits montage now presents the eight world scenes in the same
  order the player traversed the worlds when World Order randomization is on
  (Dark Land still closes the sequence). Each world's picture, sprites,
  palette, and graphics keep their original pairing — only the order changes.
- The credits mini-maps are redrawn from the randomized overworld: each world's
  little top-down map now shows a (randomly chosen) page of that world's actual
  randomized map — real terrain, paths, and level / fortress / pipe / toad-house
  markers — in the world's own palette. The picture frame, sprites, and colors
  are untouched; only the map inside each frame is regenerated. World 8 is framed
  on Bowser's castle (the finale), showing its randomized dark-world approach.
- Each credits scene's "WORLD n" caption is renumbered to match the new
  progression order, so the first world shown reads "WORLD 1", the second
  "WORLD 2", and so on (the world's name and theme are unchanged).
- Credits mini-maps now draw hand-trap slots with a ring node marker (the
  spade/bonus-game tile) instead of a stray straight path segment.

## [0.12.4] - 2026-07-17

### Added

- New shipped visual patch **Baldman Bros** by Dr. Trash Panda
  (<https://www.twitch.tv/doctor_tp>), selectable in the web app's Visual
  Patch picker.

## [0.12.3] - 2026-07-16

### Added

- **Remove Flashing** (MaCobra52): a Visual option that suppresses the
  full-screen palette flash/fade animation for photosensitive-safe play. On by
  default; not encoded in the flag key and consumes no RNG. Turn it off with
  `--keep-flashing` on the CLI.

### Fixed

- **Fire enemies stay dead** (MaCobra52's "Tail Enemies don't respawn", always
  on): Fire Chomp and Fire Snake no longer respawn after you defeat them and
  scroll them off-screen and back. ("Tail" in the patch name refers to these
  fire-trail enemies — nothing to do with the Raccoon/Tanooki tail.)

## [0.12.2] - 2026-07-16

### Changed

- Wild injections reworked to be level-centric (driven by the node catalog
  instead of raw enemy pointers). Chasers are now placed into real action
  levels only: **fortresses, airships and Bowser are excluded by type**, so a
  chaser can no longer turn up in a boss room. A level is never given a chaser it
  already has (fixes a second Angry Sun stacking onto 2-Quicksand and breaking
  it), shared enemy sets inject at most once, and injections now write to the
  correct enemy-data location (the old path was offset by 0x10, which could
  corrupt a level). Suns still spawn on screen 0. **Boss Bass is dropped from the
  injection pool** — it's a water-class enemy, so the enemy shuffle reshuffled an
  injected one away; injections are now Lakitu + Angry Sun, weighted ~2:1 toward
  the sun since Lakitu is the harder chaser. An injected Lakitu's height is
  randomized between the replaced enemy's spot and a raised height, so it isn't
  always at the harder low position.
- Wild injections roll more often (~15% → ~40% per level) so a seed lands
  noticeably more Lakitu / Angry Sun chasers.

## [0.12.1] - 2026-07-15

### Changed

- Wild injections (Lakitu / Angry Sun / Boss Bass) are no longer placed in any
  level segment that contains a Boom-Boom, so a level-wide chaser can't turn up
  in a fortress boss room.

### Fixed

- Wild-injected Angry Suns no longer get stuck idling in the background (which
  could also stop a level's goal card from spawning, making the level
  uncompletable). Injection used to leave the sun at the replaced enemy's
  position — usually deep in the level — but with Early Sun on, the sun only
  attacks if it spawned on the first screen. Injected suns are now seeded at the
  vanilla screen-0 spawn so they engage as intended.
- The "Oops all Anchors" (`anchor_visuals`) toggle is now encoded in the
  shareable flag key, so turning it on/off actually changes the key and the
  option round-trips when a key is loaded. Previously it was silently dropped
  from the flag key (flag-key version bumped to 25). Also fixed the web UI so
  applying a flag key with the toggle off actually clears it — the option was
  marked as not-in-flag-key, so `applyOptions` skipped it and left a
  previously-enabled checkbox on.

## [0.12.0] - 2026-07-12

### Fixed

- Lobby Shuffle no longer crashes when a level whose interior is a vertical
  shaft (7-1, 7-6) or a door room (2-Pyramid) is entered through another
  level's front pipe. Those interiors carry an out-of-range pipe-exit
  direction that vanilla only ever reaches by falling in or through a door;
  the shuffle now normalizes the donated direction to a valid pipe exit so
  the player lands correctly instead of crashing.

### Changed

- Overworld level placement is less linear: the weight biasing levels onto the
  main start→airship route was halved (1.5 → 0.75), so fewer forced levels get
  glued back-to-back along the critical path. Average run of consecutive
  must-play levels drops from ~2.1 to ~1.8 (in line with the reference SMB3
  randomizer) while levels still favor the route over dead-end spurs.
- Overworld pipe routing in multi-island worlds now grows a chain outward from
  the start, bridging the nearest unreached island each step, instead of always
  piping the start island straight to the goal island. Worlds like 7 and 8
  (5-7 islands) now route the player through the intermediate islands as
  intended rather than collapsing the journey into one jump; connectivity is
  still guaranteed (a direct link to the goal is used only as a last-pipe
  fallback).
- Overworld "spare" pipes (those beyond what island connectivity requires) are
  now placed after levels are laid out, so each one is aimed to skip a run of
  forced levels instead of being scored on spatial spread alone. Fewer pointless
  pipe loops, more genuine shortcuts (pipes now skip ~60% more levels), and a
  shorter average forced-level run (~1.8 → ~1.4). Every world keeps its vanilla
  pipe count; connectivity pipes are unchanged.
- World 8's showcase bridges are gated out (as a fortress lock) more often: at
  least one bridge is out in ~99% of seeds (was ~80%) and two in ~30% (was ~6%),
  with a rare ~0.08% chance all four are out at once. Pure lock-placement bias;
  connectivity and beatability are unaffected.
- Lobby Shuffle pool grows to 11 with the 2-Pyramid bonus rejoining (its
  pipe-exit crash is fixed above).
- Garbled enemy sprites in levels with player-chasing enemies: Lakitu, the
  Angry Sun, and the Big Berthas (vanilla, wild-picked, or wild-injected)
  now pin their graphics page across the whole level instead of just their
  own screen, and wild injections check the entire enemy segment (including
  levels that share its data).
- Garbled enemy sprites in levels with cannons and spawner pipes: the cannon
  fire family now counts toward graphics-page compatibility — cannonball and
  bob-omb cannons force their page level-wide (matching how the game engine
  reloads it every frame), goomba pipes and Bill cannons account for the
  page their spawned enemies need, and cannon shuffle picks respect the
  pages already committed around them.

## [0.11.2] - 2026-07-10

### Added

- **Lobby Shuffle** (off/on/maybe, `--antechamber-shuffle`) — the ten
  levels that open with an entry area whose pipe leads into the level
  itself (4-3, 5-2, 5-3, 6-6, 6-9, 7-1, 7-4, 7-5, 7-6, 7-7) get their
  interiors randomly permuted, so one level's entrance can drop into
  another's interior. The level then plays out through that interior's
  vanilla ending; map completion still credits the tile you entered from.
- 34 new king rescue quotes: 26 suit-specific (9 frog, 8 raccoon, 9 hammer)
  plus eight standard quotes.

### Changed

- Wandering map bros now avoid stepping onto hand-trap tiles entirely
  (previously they stepped on and immediately marched off again).

### Fixed

- Wandering Hammer Bros can no longer land on beaten piranha-plant or W8
  army map nodes, which let the player replay the beaten level by touching
  the bro.

### Removed

- The no-op `--shuffle-pipes` and `--shuffle-airships` CLI flags — both
  features are on by default; use `--no-shuffle-pipes` /
  `--no-shuffle-airships` to disable them.

## [0.11.1] - 2026-07-09

### Added

- **Piranha Shuffle** (off/on/wild, `--piranha-shuffle`) — frees the two W7
  piranha plant levels (7-P1/7-P2) into the level shuffle pool. On: their
  plant sprites travel with them, guarding whichever slot they land on
  (auto-starts on step, poofs when beaten, vanilla style). Wild: the plants
  scatter instead — one lands on a random level slot in each world. The
  plant levels' treasure chests now carry their own item (randomized with
  chest items), so they reward correctly no matter how they're entered.

## [0.11.0] - 2026-07-08

### Added

- **Player color picker** — choose Mario's color from a NES palette grid in
  the web app (or `--player-color <hex>` in the CLI); Luigi and the power-up
  suits get matching colors derived from the pick, keeping the vanilla
  brother contrast and natural skin tones. Random (the default) now rolls a
  random color through the same matching-wardrobe scheme instead of the old
  fully-independent byte picks. Composes with the visual re-skin patches:
  the scheme anchors on the character's current colors, so picking works
  the same on Luigi-35th, Peach, and Dr. Mario re-skins.

### Changed

- **Palette options reorganized into "Player colors" and "World colors"** —
  the old Palettes / Themed per-tileset / Player color trio is now two
  independent toggles: Player colors (the wardrobe: off = vanilla outfits,
  random, or a picked color) and World colors (themed level/enemy/map
  recoloring). Themed world colors no longer require player colors to be
  on, and turning them on no longer re-rolls the wardrobe.
- **Themed palettes: context-aware color themes + wider coverage** — themed
  palette randomization now applies subtle, context-aware hue shifts on top
  of the variant swap: each context (plains, water, fortress, desert,
  lava, maps, ...) rolls its own small shift (at most 2 steps on the NES
  hue wheel) from a per-context allowed set, so water stays watery, lava
  stays warm, and skies never go magenta. Brightness is never changed, so
  visibility is preserved. Coverage extended to the W6/W7 overworld maps,
  the slot-table tail (lava/Bowser quartets), the 0x36E20 palette pool,
  and stragglers past slice 4 — 118 new curated positions plus 324
  rotate-only positions that previously stayed vanilla.

## [0.10.3] - 2026-07-07

### Fixed

- **Airship-lock patch corrupted 4-4's sub-area** — removed a dead always-on
  write (`A9 01 EA` at `0x1FABC`) that was intended to keep the airship from
  moving. The offset actually landed in the middle of level 4-4's sub-area
  layout data, so entering that sub-area black-screened. The write did nothing
  for airship behavior — the mobile airship is a live map-object the builder
  never spawns (the airship is placed as a static tile), so there is nothing to
  lock — and removing it fixes the crash with no behavior change.

## [0.10.2] - 2026-07-05

### Fixed

- **Hold-left airship entry** — holding Left while entering an airship no longer
  spawns Mario out over the pit and kills him (seen with autoscrollers disabled).
  Applies MaCobra52's "Hold left fix" as an always-on bugfix.

## [0.10.1] - 2026-07-05

### Fixed

- **Start↔Airship swap — death respawn** — in a swapped world, dying with lives
  remaining in a level on a different overworld page than the swapped start no
  longer strands Mario on a blank tile with the map drawn on the wrong screen.
  The engine's "skid back from afar" restores the camera from a secondary scroll
  backup the swap scaffolding never seeded, so it scrolled to page 0; it is now
  seeded (at both Map Init and the game-over finalize) so the skid scrolls to the
  real start page.

### Changed

- **Start↔Airship swap — start framing** — a swapped start on a non-zero screen
  is now centered half a screen back instead of pinned at the left edge of its
  page, so the surrounding map is visible and the camera no longer auto-pans on
  arrival. Page-0 / unswapped worlds are unaffected.

## [0.10.0] - 2026-07-01

### Added

- **Randomized Boom-Boom stomp counts** — each fortress's Boom-Boom now takes a
  random 1–5 stomps to defeat (per-fortress, distinct within each world) instead
  of the fixed 3. On by default; disable with `--keep-boomboom-stomps`. Fireball
  defeats are unaffected.
- **β9 Tornado** — when beta stages are included (`--include-beta-stages`), one of
  the β9 beta stage's three Fire Chomps is randomly turned into a Tornado (borrowing
  the World 2 quicksand Tornado's height).

## [0.9.5] - 2026-07-01

### Fixed

- **Randomized Koopalings — ring graphics** — the moved ring attack now loads
  its own sprite CHR page on whichever body carries it, so the ring no longer
  renders as garbled tiles. Also fixes the reverse case where the (no-longer-
  ring) Wendy identity drew a garbled wand blast.

## [0.9.4] - 2026-06-30

### Changed

- **Randomized Koopalings — ring attack** — with random Koopalings on, Wendy's
  ring attack (ring projectile + firing cadence + straight aim) now rides a
  random Koopaling identity's body instead of always Wendy. There's still
  exactly one ring boss; only which body carries it is randomized.

## [0.9.3] - 2026-06-30

### Changed

- **Randomized Koopalings — heavy physics** — with random Koopalings on, the
  heavy-physics effect (enhanced gravity, floor-shake, player paralysis) is now
  reassigned to two random Koopaling identities instead of always Roy and
  Ludwig, so a differently-shaped boss can carry the crushing feel.

## [0.9.2] - 2026-06-28

### Fixed

- **4-1 hazard placement** — the three Big Red Troopas each sit on a small
  platform Mario must land on to move forward; in Wild enemy mode they could be
  swapped to a hazard (Thwomp/Ptooie/nipper/lotus/hotfoot), forcing an
  unavoidable hit. Those spots are now hazard-protected.

## [0.9.1] - 2026-06-27

### Changed

- **Level spread across worlds** — levels are distributed by compressed capacity
  (`capacity^0.5`) instead of straight proportional, so the densest worlds (Ice,
  Desert) no longer hoard levels and the emptier ones (Giant, Pipe, Dark) fill
  out, without forcing every world to the same count. The leftover from rounding
  is now placed in random worlds for a little per-seed variety. The old
  World 6-specific level cap is gone — the level-spread scoring's density penalty
  handles clumping, and measured clumping is actually lower at the new spread.

### Fixed

- **Overworld connectivity** — pipe placement could occasionally strand a
  world's airship/Bowser behind an unreachable region (most often Giant Land),
  producing an unbeatable world. The island-connect step now refuses to spend a
  pipe on a dead-end that doesn't lead toward the target, and will lift the
  start-adjacent no-pipe restriction when that's the only way to keep the world
  completable. This also subsumes the old World 3 start↔airship-swap pipe
  special-case, which has been removed.

### Removed

- **Remove Rocks** is no longer an option — path-blocking rocks (W2 secret path,
  W3 boat dock, W4 pipe shortcut) are always cleared, since the overworld builder
  depends on those tiles being open for connectivity. (Adding extra
  hammer-breakable shortcut rocks remains a separate option.)

## [0.9.0] - 2026-06-25

The first cut: a baseline of notable changes since the project began. It
summarizes feature areas rather than every commit — see `git log` for the
full history.

### Added

- **Shuffle HammerBro Locations** (`--no-shuffle-hammer-bros` to disable; on by
  default, issue #20) — the wandering Hammer Bro encounters are spread across all
  worlds (random 1-3 per world, 15 total, with light anti-clustering) instead of
  their fixed vanilla spots, and each carries its reward item. The Dark World
  keeps at most one, and a couple of map-object slots stay free in every world so
  level-triggered white mushroom houses can still appear. A feature-dense world
  with no spare path tile may get fewer, with its share spilling elsewhere.
- **Random Fire Flower** (`--fire-flower off|on|wild`, issue #22) — an in-level
  Fire Flower still looks the same but grants a power state derived
  deterministically from a seed salt (the shuffled starting world), the current
  world, the level, and the flower's screen, instead of always Fire. `on`
  substitutes among Fire/Frog/Tanooki/Hammer; `wild` also allows the Small/Big
  downgrades. Same seed always gives the same suit for a given flower; the
  mapping rotates per seed when world-order shuffle is enabled.
- **Overworld builder pipeline** — the core randomization system. A
  four-phase pipeline (catalog → pickup → build → write) that re-lays each
  world: assigns levels to map slots via BFS-ordered placement, places
  fortresses with locks, distributes pipes, and tags hammer-bro slots while
  enforcing connectivity.
- **Start ↔ airship swap (SAS)** — per-world option that swaps the start tile
  with the airship, including engine scaffolding, death-respawn handling, and
  game-over finalize.
- **Troll-pipe level slots** — disguise level slots as pipe tiles (`0xBC`),
  one candidate per world W2–W8.
- **Hand-trap level slots** — visible grabbing-hand tiles (`0xE6`) with a 100%
  grab.
- **Cross-world shuffles** — Toad Houses and spade games shuffled across
  worlds; world progression order shuffle.
- **Segment composers** — `segment_writer` foundation plus the Bowser-castle
  and 5-F2 podoboo-gauntlet composers for safe, X-sorted enemy-segment edits.
- **Enemy randomization** — within-class enemy swapping, a Wild piranha pool
  (self-contained, with Rocky Wrench and directional fire jets), and an
  expanded hazard taxonomy.
- **Quality-of-life flags** — Faster Frog, Limit Bro Movement, MaCobra
  tail-attack patches, and lives/drawbridge tweaks.
- **More hammer rocks** (`--more-hammer-rocks off|on|maybe`) — adds
  hammer-breakable rock shortcuts by the W1 toad house and in W8. (Replaces the
  earlier W1-only "W1 hammer rock" flag.)
- **8s are Wild** (`--eights-are-wild off|on|maybe`) — opens up World 8 (Dark
  World) with a canoe on screen 0 and extra paths on screen 2. The W8 screen-3
  water/bridge approach is now always present, independent of this flag.
- **Tri-state (off/on/maybe) flags** — seed-hidden options that resolve via a
  dedicated RNG substream.
- **Cosmetic options** — palette randomization, "Oops all Anchors" anchor
  visuals, title-screen seed-hash icons with seeded menu music, randomized
  king rescue quotes, and bundled visual patches (Super Princess Peach,
  Super Toad, Dr. Mario Bros 3, and others).
- **Web app** — browser frontend with grouped options
  (Map/Enemies/Bosses/Items/Player/Cosmetic), Off/On pills, presets, and
  sprite-sheet icons.
- **Tooling** — `tools/rom_map.py` ROM map generator with diagnostic modes,
  plus `/level`, `/tile`, and other lookup helpers; ROM Rev 1 CRC
  fingerprint and upload-time validation.

### Changed

- Airship lock is now always on (the **Remove Anchor** / `--no-airship-lock`
  option is removed): anchors always become random power-ups and airships always
  stay put after a loss instead of moving. Flag-key version bumped to 20.
- Fortress FX visibility checks use Mario's position and the real per-world
  FX slot (derived from `FortressFX_W1_W8[...]`) rather than `$0745`
  directly.
- Bullet-bill class points at cannon IDs (`0xBC`/`0xBD`) with asymmetric Wild
  counts that never exceed vanilla.
- Pipes are forbidden adjacent to start/target tiles to eliminate
  trivial-bypass worlds.
- Option tooltips show contributor credits on their own line, linked to the
  contributor (MaCobra52 credited across the features he authored).

### Fixed

- Level-data walk no longer overruns the PRG bank into the desert metatile
  table.
- Canoe edges are scoped to their own world in the overworld walker, with a
  stateful required-progression analyzer.
- SAS game-over continue softlock when the start is on a non-zero overworld
  page; W3 fixed-pipe partner biased so the SAS start can reach the airship.
- Wild piranhas keep their hitbox/visibility correct when shuffled into other
  slots; piranhas are never replaced by upward-firing hazards.
- Numerous per-level enemy protections (4-F1 narrow hallway, 7-F2 boss room,
  7-5 walkways, 8-1 Boo, 8F Roto-Discs) so randomized hazards can't block
  required paths.

[Unreleased]: https://github.com/hamstringquestionable/smb3-rs/commits/main
