# cc_limit

A thin status bar that sits at the top of your screen and shows how much of your current Claude session you've used.

![bar example](https://placeholder)

## What it does

- Shows your current 5-hour Claude session usage as a thin colored bar.
- Tells you how long until your session resets.
- Sits above all other windows (yes, even fullscreen apps).
- Reserves its strip of screen so maximized windows stop neatly below it instead of being covered.

## How to start it

Double-click `cc_limit.exe`.

That's it. The bar appears at the top of your main screen, and a small icon (the `c` logo) appears in your system tray (bottom-right corner of Windows, next to the clock).

It will use whatever you're already logged into Claude Code with — no extra sign-in needed.

## Reading the bar

| What you see | Meaning |
|---|---|
| Orange fill | You're under 70% used. Plenty left. |
| Blue fill | You're between 70% and 90% used. Worth pacing yourself. |
| Red fill | You're over 90% used. Wrap up soon. |
| Text on the right | `42% used · resets in 3h 18m` |

The bar refreshes every minute on its own. If something goes wrong it'll keep showing the last good value while it retries quietly in the background.

## Controls

**Right-click the bar** or **left- or right-click the tray icon** to open the menu:

- **Refresh now** — force an immediate update.
- **Choose screen** — pick which monitor the bar lives on. Your choice is remembered.
- **Quit** — close the app.

## Multiple monitors

The first time you run it, the bar appears on your primary screen. To move it:

1. Click the tray icon.
2. Hover **Choose screen**.
3. Pick a monitor from the list.

It'll move immediately and remember your choice next time you launch it.

## Starting it automatically with Windows

If you want the bar to come back every time you log in:

1. Press **Win + R**, type `shell:startup`, press Enter.
2. Drag `cc_limit.exe` (or a shortcut to it) into the folder that opens.

Done. It'll start with Windows from now on.

## Closing it

Right-click the bar or the tray icon → **Quit**.

If anything ever gets stuck, you can also use Task Manager → find `cc_limit` → End task.

## Where it saves things

The app stores a tiny config file (which screen you picked) in your user folder, under `AppData\Local\cc_limit`. You don't normally need to touch it.

## Troubleshooting

**The bar shows the same number for a long time.**
It refreshes once a minute. If it really is stuck, right-click → Refresh now.

**The bar isn't visible.**
It might be on another monitor. Click the tray icon → Choose screen, and pick the one you're looking at.

**Maximized windows still cover the bar.**
Quit and relaunch — sometimes Windows needs a fresh start for the reserved-space registration to take effect.

**Nothing happens when I run it.**
Make sure you've signed into Claude Code at least once on this machine. The bar reads your existing login.
