<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";
  import TagPicker from "./TagPicker.svelte";
  import TextPoolEditor from "./TextPoolEditor.svelte";
  import ContentGroupsEditor from "./ContentGroupsEditor.svelte";
  import WebLinksEditor from "./WebLinksEditor.svelte";
  import TabStrip from "./TabStrip.svelte";
  import { flushBehaviourSave, scheduleBehaviourSave } from "./behaviourSave.js";

  type Tab =
    | "groups"
    | "captions"
    | "prompts"
    | "notifications"
    | "subliminals"
    | "web_links"
    | "wallpaper";

  const tabs: { id: Tab; label: string }[] = [
    { id: "groups", label: "Content Groups" },
    { id: "captions", label: "Captions" },
    { id: "prompts", label: "Prompts" },
    { id: "notifications", label: "Notifications" },
    { id: "subliminals", label: "Subliminals" },
    { id: "web_links", label: "Web Links" },
    { id: "wallpaper", label: "Wallpaper & Splash" },
  ];

  let activeTab = $state<Tab>("groups");

  onMount(async () => {
    if (store.behaviour === null) store.behaviour = await api.getBehaviour();
  });

  onDestroy(() => {
    flushBehaviourSave();
  });
</script>

<div class="flex-1 min-h-0 p-6 flex flex-col gap-4 w-full max-w-4xl mx-auto">
  <h2 class="text-base font-semibold text-text shrink-0">Content</h2>

  {#if store.behaviour === null}
    <p class="text-sm text-muted">Loading…</p>
  {:else}
    <TabStrip {tabs} active={activeTab} onselect={(id) => (activeTab = id as Tab)} />

    <div class="flex-1 min-h-0 overflow-y-auto">
      {#if activeTab === "groups"}
        <ContentGroupsEditor />
      {:else if activeTab === "captions"}
        <TextPoolEditor
          title="Captions"
          description="Shown on popups. Empty tags apply to any media; tagged captions only show on popups of matching media."
          pool={store.behaviour!.content.captions}
          idPrefix="caption"
        />
      {:else if activeTab === "prompts"}
        <div class="flex flex-col gap-3">
          <TextPoolEditor
            title="Prompts"
            description="Text-input questions. Not matched to media — tags only narrow eligibility via content groups/Experience tag sets."
            pool={store.behaviour!.content.prompts}
            idPrefix="prompt"
          />
          <label class="flex flex-col gap-1">
            <span class="text-xs text-muted font-medium">Submit button label</span>
            <input
              bind:value={store.behaviour!.content.prompt_settings.submit_label}
              oninput={scheduleBehaviourSave}
              placeholder="Submit"
              class="px-2 py-1.5 rounded border border-border bg-surface text-text text-sm w-48
                focus:outline-none focus:border-accent"
            />
          </label>
        </div>
      {:else if activeTab === "notifications"}
        <TextPoolEditor
          title="Notifications"
          pool={store.behaviour!.content.notifications}
          idPrefix="notification"
        />
      {:else if activeTab === "subliminals"}
        <TextPoolEditor
          title="Subliminals"
          description="Brief flashed text overlays."
          pool={store.behaviour!.content.subliminals}
          idPrefix="subliminal"
        />
      {:else if activeTab === "web_links"}
        <WebLinksEditor />
      {:else if activeTab === "wallpaper"}
        <div class="flex flex-col gap-6">
          <section class="flex flex-col gap-2">
            <div>
              <h3 class="text-sm font-semibold text-text">Wallpaper</h3>
              <p class="text-xs text-muted">
                Tags identifying wallpaper media. Leave empty to disable engine-managed wallpaper.
              </p>
            </div>
            <TagPicker tags={store.behaviour!.content.wallpaper_tags} id="wallpaper-tags" />
          </section>

          <section class="flex flex-col gap-2">
            <div>
              <h3 class="text-sm font-semibold text-text">Splash</h3>
              <p class="text-xs text-muted">
                Tags identifying a startup splash image. Leave empty to disable it.
              </p>
            </div>
            <TagPicker tags={store.behaviour!.content.splash_tags} id="splash-tags" />
          </section>
        </div>
      {/if}
    </div>
  {/if}
</div>
