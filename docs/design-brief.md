# hydra design brief

hydra runs your AI coding agents and terminals side by side, keeps them running when you close
the window, and tells you which one needs you. Part 1 explains how it works in plain terms.
Part 2 is a brief for a designer. The prompt to paste into a design tool is in
`design-prompt.txt` next to this file.

---

## Part 1 · How hydra works

### Five things to know

They nest: a workspace holds tabs, a tab holds panes, and a pane runs a shell or an agent.

| Thing | What it is |
|---|---|
| **Workspace** | One project. It has a home folder, a colour, and everything you opened for it. Switching workspace swaps the whole screen. |
| **Tab** | A page inside a workspace, like browser tabs. Use them to keep "agents" separate from "server logs". |
| **Pane** | One terminal on the screen. Split a tab into as many panes as you like. Each pane remembers which folder it's in. |
| **Agent** | Claude, Codex or another AI tool running in a pane. hydra spots them automatically and tracks whether they're working or waiting on you. |
| **Worktree** | A second copy of the same git repo on a different branch, so two agents can change code without stepping on each other. Each one can be its own workspace. |
| **Leader key** | `Ctrl+Space`, then one letter, does any command. Press it and pause to see every option. |

### The screen today

```
hydra  2 workspaces        shop ▲1 ●1  │ 1 claude  2 shell  +                main ±3
                         ╭ ● claude working ────────────╮╭ shell · shop ──────────╮
  shop                   │ > fixing the checkout test   ││ PS C:\dev\shop>        │
 ▌ main  ±3              │                              ││                        │
     ● claude fix flaky… │                              ││                        │
     ▲ claude needs you  ╰──────────────────────────────╯╰────────────────────────╯
 ▌ feat-x                ╭ ▲ claude ──────╮╭ ● claude ──────╮╭ ◆ codex ─────╮
     ◆ codex  done       │ shop needs you ││ shop  working  ││ feat-x done  │
   ○ feat-y              │ add coupons    ││ fix flaky test ││ refactor     │
   + new worktree        ╰────────────────╯╰────────────────╯╰──────────────╯
 + new workspace
 sidebar (map)           panes (work) above · agent dock (who needs you) below
```

- **Sidebar:** repos, their workspaces and worktrees, and the agents and shells in each. Click a row to go there.
- **Top bar:** the current workspace, its tabs, and its git branch with the count of changed files (`±3`).
- **Panes:** your terminals, tinted in the workspace's colour so you always know which project you're typing into.
- **Agent dock:** one card per agent across every workspace, most urgent first, with the last thing you asked it.

### What the agent symbols mean

| Symbol | State | Meaning |
|---|---|---|
| ▲ | needs you | Waiting for your answer or permission. Nothing happens until you respond. This is the one to act on. |
| ◆ | done | Finished while you were looking elsewhere. Clears once you look at it. |
| ● | working | Busy on your task (spins while active). Leave it alone. |
| ○ | idle | Ready for a new task. |

### How to do the common things

Every shortcut is the leader (`Ctrl+Space`), let go, then the key.

| I want to… | Do this |
|---|---|
| Make a workspace for the project I'm in | `cd` there, then `M` |
| Make a workspace for some other folder | `N` or click *+ new workspace* |
| Give an agent a task | `q`, type it, Enter |
| Go to the agent that needs me | `a` or click its card |
| Find anything (command, project, agent) | `Space`, type |
| Work on a branch in its own copy | `W`, type a branch name |
| Split the screen | `%` right · `-` down |
| Search or copy earlier output | `/` search · `[` copy mode |
| Change the look or the leader key | `S` settings |
| Step away, leave everything running | `d` (or just close the window) |

### What survives what

| If you… | Agents and shells |
|---|---|
| Close the window or press `d` | **Keep running** |
| Reboot, or run `hydra kill-server` | **Come back**: layout restored, agents resume their last conversation, shells start fresh |
| Close every pane yourself | **Gone**: next start is a clean slate |

---

## Part 2 · Brief for a designer

### The problem

hydra works, but its owner, who is also its main user, finds it overwhelming. Five concepts, two
places listing agents, a dozen shortcuts and several popups all arrive at once, and the screen
doesn't say what to do next. The goal is a screen that explains itself: a first-time user should
know where they are, what needs them, and how to start something new, without reading docs.

### Where it's confusing today

- **Workspace vs folder vs pane location.** You can `cd` a pane somewhere else, but the workspace stays put. It's unclear which one "where I am" means.
- **Nothing says how to start.** Creating a workspace or worktree is a hidden shortcut. Empty states don't invite the next step.
- **Agents shown twice.** The sidebar and the dock both list agents. It's unclear which to look at.
- **Symbols carry the meaning.** ▲ ◆ ● ○ and `±3` are compact but cryptic until learned.
- **Messages hide in a corner.** Errors and confirmations appear briefly in the top right and are easy to miss.
- **Worktrees are unfamiliar.** Many users don't know the concept. The UI should teach it in place, not assume it.
- **It reads like herdr.** The owner wants hydra to have its own identity, not to look like a copy of another tool.

### Constraints

- It's a terminal app: everything is drawn in a grid of monospace character cells. No images, no anti-aliased shapes, no variable fonts or sizes. One cell is one character.
- Available materials: 24-bit colour (foreground and background per cell), bold, dim, italic, underline, Unicode box-drawing and block characters, and Nerd Font glyphs.
- Must work from about 100×30 cells up to 300×80, and degrade gracefully when narrow.
- Keyboard first (leader key + one letter), but every action must also be reachable by mouse.
- The panes show other programs' output in their own colours. hydra's chrome must not clash with it.
- Light and dark terminal themes both exist. Themes, colours, icons and layout are all user-configurable, so the design should be a system, not one fixed skin.
- Primary platform: Windows Terminal on Windows 11. Also macOS and Linux.

### What to deliver

- The main screen at small (100×30), medium (160×45) and large (240×65) sizes.
- First run with nothing open, and a workspace with no agents yet: screens that teach the next step.
- Each agent state, and how "needs you" grabs attention without being noisy.
- The popups: shortcut list, command palette, quick prompt, pane menu, settings, worktree picker.
- A decision on sidebar vs dock: one home for agents, or a clear job for each.
- A small glyph set and colour roles (status, workspace colours, chrome), with light and dark versions.
- Mockups as monospace text, or a design-tool frame on a strict character grid, so they can be built exactly.

Worth studying: lazygit, zellij, k9s, btop and nebula for terminal UI patterns. herdr only as a
"not this" reference.
