# Muxy product model

This directory describes the product concepts behind Muxy.

The documents intentionally describe **what the product means and does**, not
how it will be implemented.

## Product in one paragraph

Muxy is a blazing fast and memory efficient terminal multiplexer that supports 
organizing projects and working in tabs made from one or more panes. 
Every project belongs to a server and points to a directory there. The server
owns shared project metadata and terminal sessions; each client owns its
workspaces, tabs, panes, and presentation.

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
| Main app | The desktop client that stores workspaces and presentation state and directs work to the appropriate server. |
| Server | A separate process that owns projects, their shared metadata, terminal sessions, and server settings. Clients own presentation. |
| `muxy` executable | The bundled or standalone command-line and terminal client. It connects to the separate `muxy-server` executable. |
| Workspace | A reusable, app-level grouping used to filter top-level projects. Workspaces may overlap. |
| Project | An independently identified server record pointing to one directory, with shared name, icon, and color. Each client keeps its own tab set for it. |
| Project location | The combination of a server and directory. It is not a project's identity and does not need to be unique. |
| Top-level project | A project with no parent. It represents the main project directory and appears in the sidebar. |
| Home project | Each server's protected top-level project, pointing at that server user's home directory and owning ad-hoc sessions. It cannot be deleted, have worktree children, or belong to workspaces. |
| Project type | An optional classification for specialized projects. Ordinary projects have no type. |
| Worktree project | A child project with `type = worktree`, its own directory, and a `parent_id` pointing to its top-level project. |
| Tab | A client-owned untitled container associated with one project, holding panes in a saved layout. On desktop it shows the window-focused pane's title, or its first pane's title when inactive. |
| Pane | One typed unit of content within a tab, with its own title and content and details. Pane types are either app-only, such as a web view, or server-bound, such as a terminal. |
| Session | A running terminal process belonging to exactly one server-owned project and identified by a server-generated ID. It outlives the app and ends only when its process exits, a client ends it, or the server stops. |
| Attach | The act of a client connecting to a session to receive its output and send input. Any number of clients may be attached at once; detaching never affects the session. |
| History | The output a server retains for a session up to a retention limit. A recent window is sent on attach and older pages are available on request. |

