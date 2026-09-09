# Muxy product model

This directory describes the product concepts behind Muxy.

The documents intentionally describe **what the product means and does**, not
how it will be implemented.

## Product in one paragraph

Muxy is a blazing fast and memory efficient terminal multiplexer that supports 
organizing projects and working in tabs made from one or more panes. 
Every project points to a directory on either the current device or a 
remote server and owns its own tabs.

The technical design that implements this model is in
[../tech/README.md](../tech/README.md).

## Reading order

1. [Product model](./product-model.md) — definitions, relationships, and
   ownership rules.
2. [App model](./app-model.md) — current-device and remote-server boundaries,
   followed by the visible navigation model.
3. [Server model](./server-model.md) — what a server owns, how sessions
   live, and the capability areas it provides.

## Core vocabulary

| Term | Meaning |
| --- | --- |
| Main app | The user-facing app that stores organization and presentation state and directs work to the appropriate server. |
| Server | The current-device or remote execution context that provides raw capabilities such as terminal sessions and Git operations. It runs as its own process, knows nothing about projects, and holds no app policy. |
| Workspace | A reusable, app-level grouping used to filter top-level projects. Workspaces may overlap. |
| Project | An independently identified app record that points to one directory on one server and owns a tab set. |
| Project location | The combination of a server and directory. It is not a project's identity and does not need to be unique. |
| Top-level project | A project with no parent. It represents the main project directory and appears in the sidebar. |
| Home project | The always-present top-level project on the current-device server, pointing at the OS home directory. It cannot be deleted, cannot have worktree children, and does not belong to workspaces. |
| Project type | An optional classification for specialized projects. Ordinary projects have no type. |
| Worktree project | A child project with `type = worktree`, its own directory, and a `parent_id` pointing to its top-level project. |
| Tab | An untitled container owned directly by a project, holding one or more panes in a saved layout. It shows the window-focused pane's title, or its first pane's title when inactive. |
| Pane | One typed unit of content within a tab, with its own title and content and details. Pane types are either app-only, such as settings, or server-bound, such as a terminal. |
| Session | A running terminal process owned by a server and identified by a server-generated ID. It outlives the app and ends only when its process exits, a client ends it, or the server stops. |
| Attach | The act of a client connecting to a session to receive its output and send input. Any number of clients may be attached at once; detaching never affects the session. |
| History | The output a server retains for a session up to a retention limit. A recent window is sent on attach and older pages are available on request. |

