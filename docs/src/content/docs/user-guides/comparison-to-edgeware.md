---
title: Comparison to Edgeware++
description: Comparing the features and design decisions of the two programs
---

Lewdware was made as an alternative to [Edgeware++](https://github.com/araten10/EdgewarePlusPlus),
an improved version of [Edgeware](https://github.com/PetitTournesol/Edgeware),
which in turn was inspired by Elsavirus.

Lewdware aims to improve upon Edgeware++ in a variety of ways:

- Easier to install - Edgeware++ involves a reasonably complex and fragile
  installation process. Lewdware is distributed as a single installer on
  all platforms.
- Smaller packs - Lewdware's media compression can result in packs being five
  or ten times smaller than a corresponding Edgeware pack. Not only do packs
  take up less space on your computer, but packs also become easier to
  distribute and download. Edgeware++ also has to copy and decompress a pack to
  run it, Lewdware's custom pack format allows it to process packs in place.
- Better performance - Edgeware++ can struggle with lots of windows open.
  Lewdware can handle hundreds of windows at a time (although it will crash
  eventually).
- Modern UI - Lewdware provides a modern config UI that is easy to use.
- A pack editor - Lewdware provides a GUI pack editor, making it super easy
  to create and edit packs.
- Better scripting - While both Edgeware++ and Lewdware provide scripting
  features, Lewdware's scripting is much more fully-featured, providing APIs for
  moving and setting the transparency of windows, and allowing users to pass
  options in to modes.
- Better scripting (for developers) - While Edgeware only supports a subset
  of Lua, Lewdware runs a sandboxed version of Lua 5.5. Lewdware provides
  a CLI tool to create, test and build modes. We provide type definitions
  for the mode API, providing developers with auto-complete and type checking
  in an editor like VSCode. Lewdware allows you to split your modes into
  multiple files, which will be compressed and built into a single bundle.

However, there are many features Edgeware++ provides that Lewdware does
not. Many of these differences come from the fact that Lewdware relies
more on its scripting features (modes) to determine behaviour, which
results in less space for user configuration (unless you write your own mode).

- Edgeware++ allows you to configure in more detail how windows and other
  features are spawned (e.g. the proportion of video windows).
- Edgeware++ allows links to be opened, prompts to be shown and wallpapers
  to be changed without any scripting, since the three are included as
  options in packs.
- Edgeware++ provides a "corruption" feature, allowing packs to change
  behaviour over time without any scripting. This behaviour can be
  emulated by a Lewdware mode, but doing so is more complex.
- Edgeware++ has a "fill drive" feature that modifies files on your computer.
  Lewdware does not currently do this, for security reasons (this is potentially
  unsafe and unwanted for obvious reasons). If we do implement this, it would
  probably be a standalone program, rather than being integrated into Lewdware.
- Edgeware++ provides a Booru downloader. Lewdware does not do this because
  it would be difficult to maintain.
