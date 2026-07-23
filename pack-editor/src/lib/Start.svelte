<script lang="ts">
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";
  import { history } from "./history.svelte.js";
  import Dialog from "$ui/Dialog.svelte";
  import type { PackInfo, RecentPack } from "./types.js";
  import Button from "$ui/Button.svelte";
  import { ArrowDownTray, DocumentPlus, FolderOpen, Icon, XMark } from "svelte-hero-icons";
  import { onMount } from "svelte";

  let showUnsavedDialog = $state(false);
  let pendingInfo = $state<PackInfo | null>(null);
  let busy = $state<"new" | "open" | "import" | null>(null);
  let error = $state<string | null>(null);
  let recents = $state<RecentPack[]>([]);
  let modifierLabel = $state("Ctrl");

  onMount(async () => { recents = await api.getRecentPacks(); });

  onMount(() => {
    modifierLabel = navigator.platform.includes("Mac") ? "⌘" : "Ctrl";
    const handleShortcut = (event: KeyboardEvent) => {
      if (event.defaultPrevented || busy !== null || !(event.ctrlKey || event.metaKey) || event.altKey) return;
      const key = event.key.toLowerCase();
      if (key === "n") { event.preventDefault(); void newPack(); }
      else if (key === "o") { event.preventDefault(); void openPack(); }
    };
    window.addEventListener("keydown", handleShortcut);
    return () => window.removeEventListener("keydown", handleShortcut);
  });

  async function finishOpen(info: PackInfo) {
    const [files, tags, artists] = await Promise.all([api.getFiles(), api.getAllTags(), api.getAllArtists()]);
    store.openPack(info.name, files, tags, artists, !info.has_unsaved_changes, info.has_destination);
    history.reset(!info.has_unsaved_changes);
  }

  async function newPack() {
    busy = "new";
    error = null;
    try {
      const info = await api.newPack();
      await finishOpen(info);
    } catch (err) {
      error = `Could not create the pack. ${String(err)}`;
    } finally {
      busy = null;
    }
  }

  async function openPack() {
    busy = "open";
    error = null;
    try {
      const info = await api.openPackDialog();
      if (!info) return;
      if (info.has_unsaved_changes) {
        pendingInfo = info;
        showUnsavedDialog = true;
      } else {
        await finishOpen(info);
      }
    } catch (err) {
      error = `Could not open the pack. ${String(err)}`;
    } finally {
      busy = null;
    }
  }

  async function openRecent(recent: RecentPack) {
    busy = "open";
    error = null;
    try {
      const info = await api.openRecentPack(recent);
      if (info.has_unsaved_changes && info.has_destination) {
        pendingInfo = info;
        showUnsavedDialog = true;
      } else await finishOpen(info);
    } catch (err) {
      error = `Could not open ${recent.name}. ${String(err)}`;
    } finally { busy = null; }
  }

  async function removeRecent(recent: RecentPack) {
    error = null;
    try {
      await api.removeRecentPack(recent);
      recents = recents.filter((item) => (item.path ?? item.draft_id) !== (recent.path ?? recent.draft_id));
    } catch (err) {
      error = `Could not remove ${recent.name}. ${String(err)}`;
    }
  }

  async function onUnsavedLoad() {
    showUnsavedDialog = false;
    const info = pendingInfo!;
    pendingInfo = null;
    await finishOpen(info);
  }

  async function onUnsavedDiscard() {
    showUnsavedDialog = false;
    const info = pendingInfo!;
    pendingInfo = null;
    await api.discardChanges();
    await finishOpen({ ...info, has_unsaved_changes: false });
  }

  async function onUnsavedCancel() {
    showUnsavedDialog = false;
    pendingInfo = null;
    await api.closePack();
  }

  async function importEdgeware() {
    busy = "import";
    error = null;
    try {
      const result = await api.importEdgewarePackDialog();
      if (!result) return;
      store.openPack(result.info.name, [], [], [], false, false);
      history.reset(false);
      store.importWarnings = result.warnings;
      // behaviour.json/metadata are already written by the time this command returns (see
      // import_edgeware_pack_dialog/run_import) -- fetch it right away, no waiting on media.
      store.behaviour = await api.getBehaviour();
      store.packSaved = !result.info.has_unsaved_changes;
    } catch (err) {
      error = `Import failed. ${String(err)}`;
    } finally {
      busy = null;
    }
  }
</script>

<main class="start-screen">
  <section class="welcome" aria-labelledby="welcome-title">
    <header>
      <div class="app-mark" aria-hidden="true">
        <span class="frame f1"></span>
        <span class="frame f2"></span>
        <span class="frame f3"><span class="frame-bar"><span class="frame-dot"></span></span></span>
      </div>
      <div>
        <p class="eyebrow">Lewdware</p>
        <h1 id="welcome-title">Pack Editor</h1>
        <p class="intro">Create and organise packs, their content, and the experience they provide.</p>
      </div>
    </header>

    <div class="primary-actions">
      <article>
        <span class="action-icon" aria-hidden="true"><Icon src={DocumentPlus} /></span>
        <div class="action-copy">
          <h2>Create a new pack</h2>
          <p>Start with an empty pack, then add media and configure its behaviour.</p>
        </div>
        <Button variant="primary" onclick={newPack} loading={busy === "new"} disabled={busy !== null} title={`New pack (${modifierLabel}+N)`}>New pack</Button>
      </article>

      <article>
        <span class="action-icon" aria-hidden="true"><Icon src={FolderOpen} /></span>
        <div class="action-copy">
          <h2>Open an existing pack</h2>
          <p>Continue editing a Lewdware pack stored on this computer.</p>
        </div>
        <Button onclick={openPack} loading={busy === "open"} disabled={busy !== null} title={`Open pack (${modifierLabel}+O)`}>Open pack…</Button>
      </article>
    </div>

    {#if recents.length > 0}
      <section class="recent" aria-labelledby="recent-title">
        <h2 id="recent-title">Recent packs</h2>
        <div class="recent-list">
          {#each recents as recent (recent.path ?? recent.draft_id)}
            <div class="recent-row">
              <button class="recent-open" type="button" onclick={() => openRecent(recent)} disabled={busy !== null}>
                <span class="recent-icon" aria-hidden="true"><Icon src={FolderOpen} mini /></span>
                <span class="recent-copy">
                  <strong>{recent.name}{#if !recent.path}<span class="draft-badge">Recoverable draft</span>{/if}</strong>
                  <small title={recent.path ?? "Stored in local recovery data"}>{recent.path ?? "Backed up locally · choose a destination on first save"}</small>
                </span>
              </button>
              <button class="recent-remove" type="button" aria-label={`Remove ${recent.name} from recent packs`} title="Remove from recent packs" onclick={() => removeRecent(recent)}><Icon src={XMark} mini size="14px" /></button>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    <div class="migration">
      <span class="migration-icon" aria-hidden="true"><Icon src={ArrowDownTray} /></span>
      <div>
        <h2>Moving from Edgeware?</h2>
        <p>Import an Edgeware pack and convert it into an editable Lewdware pack.</p>
      </div>
      <Button size="compact" onclick={importEdgeware} loading={busy === "import"} disabled={busy !== null}>Import Edgeware pack…</Button>
    </div>

    {#if error}
      <div class="error" role="alert">
        <span>{error}</span>
        <button type="button" onclick={() => (error = null)}>Dismiss</button>
      </div>
    {/if}
  </section>
</main>

{#if showUnsavedDialog}
  <Dialog
    title="Unsaved changes found"
    description="This pack has unsaved changes from a previous session."
    buttons={[
      { label: "Cancel", onclick: onUnsavedCancel },
      { label: "Discard Changes", destructive: true, onclick: onUnsavedDiscard },
      { label: "Load Changes", primary: true, onclick: onUnsavedLoad },
    ]}
    onclose={onUnsavedCancel}
  />
{/if}

<style>
  .draft-badge { display: inline-flex; margin-left: 8px; padding: 2px 6px; border-radius: 999px; background: var(--ui-warning-bg); color: var(--ui-warning); font-size: 10px; font-weight: 600; vertical-align: 1px; }
  .start-screen { display: grid; min-height: 100vh; padding: 32px; place-items: center; overflow-y: auto; background: var(--ui-bg); color: var(--ui-text); }
  .welcome { width: min(680px, 100%); }
  header { display: flex; align-items: center; gap: 16px; margin-bottom: 24px; }
  .app-mark { position: relative; width: 54px; height: 42px; flex: none; margin: 12px 4px 2px 14px; }
  .frame { position: absolute; inset: 0; display: block; border: 1px solid var(--ui-border-strong); border-radius: var(--ui-radius-md); background: var(--ui-bg); }
  .frame.f1 { transform: translate(-13px, -11px); opacity: .45; }
  .frame.f2 { transform: translate(-6px, -5px); opacity: .7; }
  .frame.f3 { border-color: var(--ui-accent); background: var(--ui-surface); box-shadow: 4px 4px 0 rgb(0 0 0 / .5); }
  .frame-bar { display: flex; height: 12px; padding: 0 5px; align-items: center; border-bottom: 1px solid var(--ui-border); border-radius: var(--ui-radius-md) var(--ui-radius-md) 0 0; background: var(--ui-surface-raised); }
  .frame-dot { width: 4px; height: 4px; border-radius: 50%; background: var(--ui-accent); }
  .eyebrow { margin: 0 0 2px; color: var(--ui-muted); font-family: var(--ui-font-mono); font-size: 12px; font-weight: 700; }
  h1 { margin: 0; font-size: 24px; line-height: 1.15; letter-spacing: -.02em; }
  .intro { max-width: 520px; margin: 6px 0 0; color: var(--ui-muted); font-size: 14px; line-height: 1.45; }
  .primary-actions { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
  article { display: flex; min-height: 218px; padding: 20px; align-items: flex-start; flex-direction: column; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); }
  article:hover { border-color: var(--ui-border-strong); }
  .action-icon { display: inline-flex; width: 26px; height: 26px; margin-bottom: 17px; color: var(--ui-accent-foreground); }
  .action-copy { flex: 1; }
  h2 { margin: 0; color: var(--ui-text); font-size: 16px; line-height: 1.25; }
  .action-copy p, .migration p { margin: 7px 0 18px; color: var(--ui-muted); font-size: 12px; line-height: 1.5; }
  .migration { display: grid; margin-top: 12px; padding: 14px 16px; grid-template-columns: 22px minmax(0, 1fr) auto; align-items: center; gap: 12px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: color-mix(in srgb, var(--ui-surface) 65%, transparent); }
  .recent { margin-top: 20px; }
  .recent > h2 { margin: 0 0 8px; color: var(--ui-muted); font-family: var(--ui-font-mono); font-size: 12px; font-weight: 700; }
  .recent-list { overflow: hidden; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); }
  .recent-row { display: flex; min-width: 0; align-items: center; border-bottom: 1px solid var(--ui-border); }
  .recent-row:last-child { border-bottom: 0; }
  .recent-open { display: flex; min-width: 0; min-height: 48px; flex: 1; padding: 7px 12px; align-items: center; gap: 10px; border: 0; background: transparent; color: var(--ui-text); font: inherit; text-align: left; cursor: pointer; }
  .recent-open:hover:not(:disabled) { background: var(--ui-surface-raised); }
  .recent-open:disabled { opacity: .5; cursor: wait; }
  .recent-icon { display: inline-flex; width: 18px; height: 18px; flex: none; color: var(--ui-muted); }
  .recent-copy { display: flex; min-width: 0; flex-direction: column; gap: 2px; }
  .recent-copy strong { overflow: hidden; font-size: 13px; font-weight: 600; text-overflow: ellipsis; white-space: nowrap; }
  .recent-copy small { overflow: hidden; color: var(--ui-muted); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
  .recent-remove { display: grid; width: 34px; height: 34px; margin-right: 7px; flex: none; padding: 0; place-items: center; border: 0; border-radius: var(--ui-radius-sm); background: transparent; color: var(--ui-muted); cursor: pointer; }
  .recent-remove:hover { background: var(--ui-danger-bg); color: var(--ui-danger); }
  .recent-open:focus-visible, .recent-remove:focus-visible { outline: 2px solid var(--ui-focus); outline-offset: -2px; }
  .migration-icon { display: inline-flex; width: 20px; height: 20px; color: var(--ui-muted); }
  .migration h2 { font-size: 14px; }
  .migration p { margin: 3px 0 0; }
  .error { display: flex; margin-top: 12px; padding: 10px 12px; align-items: center; justify-content: space-between; gap: 16px; border: 1px solid var(--ui-danger-border); border-radius: var(--ui-radius-sm); background: var(--ui-danger-bg); color: var(--ui-danger); font-size: 12px; line-height: 1.4; }
  .error button { flex: none; padding: 3px 5px; border: 0; border-radius: 3px; background: transparent; color: inherit; font: inherit; font-weight: 600; cursor: pointer; }
  .error button:hover { background: color-mix(in srgb, var(--ui-danger) 12%, transparent); }
  .error button:focus-visible { outline: 2px solid var(--ui-focus); outline-offset: 2px; }
  @media (max-width: 600px) {
    .start-screen { padding: 24px 16px; place-items: start center; }
    .primary-actions { grid-template-columns: 1fr; }
    article { min-height: 0; }
    .action-copy { margin-bottom: 4px; }
    .migration { grid-template-columns: 22px 1fr; }
    .migration :global(button) { grid-column: 2; justify-self: start; }
    .draft-badge { display: table; margin: 3px 0 0; }
  }
</style>
