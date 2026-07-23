<script lang="ts">
  import { onMount } from "svelte";
  import { CodeBracketSquare, Icon, Plus, Trash } from "svelte-hero-icons";
  import Button from "$ui/Button.svelte";
  import Dialog from "$ui/Dialog.svelte";
  import EmptyState from "$ui/EmptyState.svelte";
  import Select from "$ui/Select.svelte";
  import { api } from "./api.js";
  import { history } from "./history.svelte.js";
  import { flushMetadataSave, initializeMetadataHistory, scheduleMetadataSave } from "./metadataSave.svelte.js";
  import { store } from "./store.svelte.js";
  import type { EmbeddedMode, RecommendedMode } from "./types.js";

  let modes = $state<EmbeddedMode[]>([]);
  let loaded = $state(false);
  let adding = $state(false);
  let removing = $state<EmbeddedMode | null>(null);
  let error = $state<string | null>(null);

  const recommendedValue = $derived.by(() => {
    const value = store.metadata?.recommended_mode;
    if (!value) return "auto";
    if (value === "Sandbox") return "sandbox";
    if (value === "Experience") return "experience";
    return `pack:${value.Pack.id}`;
  });
  // The timeline mode ("Sequence") shows under this pack's label when one is set, matching what
  // the player will see in the config app.
  const timelineName = $derived(store.behaviour?.experience?.label ?? "Sequence");
  const recommendationOptions = $derived([
    { value: "auto", label: `Automatic (${store.behaviour?.experience ? timelineName : "Sandbox"})` },
    { value: "sandbox", label: "Sandbox" },
    { value: "experience", label: timelineName },
    ...modes.map((mode) => ({ value: `pack:${mode.id}`, label: mode.name })),
  ]);

  onMount(async () => {
    try {
      const [loadedModes, metadata] = await Promise.all([
        api.getModes(),
        store.metadata ? Promise.resolve(store.metadata) : api.getPackMetadata(),
      ]);
      modes = loadedModes;
      store.metadata = metadata;
      initializeMetadataHistory(metadata);
      if (!store.behaviour) store.behaviour = await api.getBehaviour();
    } catch (cause) { error = String(cause); }
    finally { loaded = true; }
  });

  function setRecommendation(value: string) {
    if (!store.metadata) return;
    let recommended_mode: RecommendedMode | null = null;
    if (value === "sandbox") recommended_mode = "Sandbox";
    else if (value === "experience") recommended_mode = "Experience";
    else if (value.startsWith("pack:")) recommended_mode = { Pack: { id: Number(value.slice(5)) } };
    store.metadata = { ...store.metadata, recommended_mode };
    scheduleMetadataSave(store.metadata);
  }

  async function addMode() {
    adding = true; error = null;
    try {
      const mode = await api.addModeDialog();
      if (!mode) return;
      modes = [...modes, mode];
      history.record({
        label: `Add mode “${mode.name}”`,
      });
    } catch (cause) { error = String(cause); }
    finally { adding = false; }
  }

  async function confirmRemove() {
    if (!removing) return;
    const mode = removing;
    removing = null; error = null;
    try {
      if (recommendedValue === `pack:${mode.id}`) {
        setRecommendation("auto");
        await flushMetadataSave();
      }
      await api.removeMode(mode.id);
      modes = modes.filter((item) => item.id !== mode.id);
      history.record({
        label: `Remove mode “${mode.name}”`,
        storageBytes: mode.size,
      });
    } catch (cause) { error = String(cause); }
  }

  function formatSize(bytes: number) {
    return bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
</script>

<div class="page">
  <header>
    <div><h2 class="ui-page-title">Modes</h2><p>Bundle a Lua mode with this pack and define the options people can configure before it runs.</p></div>
    {#if loaded && modes.length > 0}
      <Button variant="primary" onclick={addMode} loading={adding}><span class="w-4 h-4"><Icon src={Plus} mini /></span> Add mode…</Button>
    {/if}
  </header>

  {#if error}<div class="error" role="alert"><span>{error}</span><button onclick={() => (error = null)}>Dismiss</button></div>{/if}

  <section class="recommendation">
    <div><h3>Recommended mode</h3><p>The config app preselects this mode when someone opens the pack. They can still choose another mode.</p></div>
    <Select class="mode-select" label="Recommended mode" hideLabel value={recommendedValue} options={recommendationOptions} onchange={setRecommendation} />
  </section>

  {#if !loaded}<p class="loading">Loading…</p>
  {:else if modes.length === 0}
    <EmptyState title="No custom modes" description="Add a built .lwmode file to distribute its Lua scripts and option specification with this pack." actionLabel="Add mode…" onclick={addMode} />
  {:else}
    <div class="mode-list">
      {#each modes as mode (mode.id)}
        <article>
          <span class="mode-icon"><Icon src={CodeBracketSquare} /></span>
          <div class="mode-copy"><h3>{mode.name}</h3><p>{[mode.author, mode.version ? `Version ${mode.version}` : null].filter(Boolean).join(" · ") || "No author or version provided"}</p><small>{mode.option_count} option{mode.option_count === 1 ? "" : "s"} · {formatSize(mode.size)}</small></div>
          {#if recommendedValue === `pack:${mode.id}`}<span class="recommended">Recommended</span>{/if}
          <Button size="compact" variant="quiet" ariaLabel={`Remove ${mode.name}`} title="Remove mode" onclick={() => (removing = mode)}><Icon src={Trash} mini size="15px" /></Button>
        </article>
      {/each}
    </div>
  {/if}
</div>

{#if removing}
  <Dialog title={`Remove “${removing.name}”?`} description="The mode’s Lua scripts and option specification will be removed from this pack. You can undo this change from the editor history." buttons={[{ label: "Cancel", onclick: () => (removing = null) }, { label: "Remove mode", destructive: true, onclick: confirmRemove }]} onclose={() => (removing = null)} />
{/if}

<style>
  .page { display: flex; height: 100%; padding: 24px; overflow-y: auto; flex-direction: column; align-items: center; }
  .page > :global(*) { width: 100%; max-width: 800px; }
  header { display: flex; margin-bottom: 18px; align-items: start; justify-content: space-between; gap: 24px; }
  header p, .recommendation p { margin: 4px 0 0; max-width: 620px; color: var(--ui-muted); font-size: 13px; line-height: 1.45; }
  .error { display: flex; margin-bottom: 12px; padding: 9px 11px; justify-content: space-between; gap: 12px; border: 1px solid var(--ui-danger-border); border-radius: var(--ui-radius-sm); background: var(--ui-danger-bg); color: var(--ui-danger); font-size: 12px; }
  .error button { border: 0; background: transparent; color: inherit; cursor: pointer; }
  .loading { padding: 36px; border: 1px dashed var(--ui-border); border-radius: var(--ui-radius-md); color: var(--ui-muted); text-align: center; font-size: 13px; }
  .recommendation { display: flex; margin-bottom: 18px; padding: 14px; align-items: center; justify-content: space-between; gap: 24px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); }
  h3 { margin: 0; font-size: 14px; } .recommendation p { font-size: 12px; }
  .recommendation :global(.mode-select) { width: 240px; flex: none; }
  .mode-list { overflow: hidden; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); }
  article { display: flex; min-width: 0; min-height: 72px; padding: 12px 14px; align-items: center; gap: 12px; border-bottom: 1px solid var(--ui-border); }
  article:last-child { border-bottom: 0; }
  .mode-icon { display: inline-flex; width: 28px; height: 28px; flex: none; color: var(--ui-accent-foreground); }
  .mode-copy { min-width: 0; flex: 1; } .mode-copy h3, .mode-copy p { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mode-copy p { margin: 3px 0; color: var(--ui-muted); font-size: 11px; } .mode-copy small { color: var(--ui-muted); font-size: 10px; }
  .recommended { flex: none; padding: 3px 7px; border: 1px solid var(--ui-border-strong); border-radius: 999px; background: var(--ui-surface-raised); color: var(--ui-text); font-size: 10px; font-weight: 600; }
  @media (max-width: 650px) { .page { padding: 16px; } header, .recommendation { align-items: stretch; flex-direction: column; gap: 12px; } .recommendation :global(.mode-select) { width: 100%; } .recommended { display: none; } }
</style>
