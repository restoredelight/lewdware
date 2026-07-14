<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";
  import TagPicker from "./TagPicker.svelte";
  import TextPoolEditor from "./TextPoolEditor.svelte";
  import ContentGroupsEditor from "./ContentGroupsEditor.svelte";
  import WebLinksEditor from "./WebLinksEditor.svelte";
  import Tabs from "$ui/Tabs.svelte";
  import { flushBehaviourSave, initializeBehaviourHistory, scheduleBehaviourSave } from "./behaviourSave.svelte.js";

  type Tab =
    | "groups"
    | "captions"
    | "prompts"
    | "notifications"
    | "subliminals"
    | "web_links"
    | "wallpaper";

  const tabs: { id: Tab; label: string; group: string }[] = [
    { id: "groups", label: "Content Groups", group: "Organization" },
    { id: "captions", label: "Captions", group: "Messages" },
    { id: "prompts", label: "Prompts", group: "Messages" },
    { id: "notifications", label: "Notifications", group: "Messages" },
    { id: "subliminals", label: "Subliminals", group: "Messages" },
    { id: "web_links", label: "Web Links", group: "Other" },
    { id: "wallpaper", label: "Wallpaper & Splash", group: "Other" },
  ];

  const sectionInfo: Record<Tab, { title: string; description: string }> = {
    groups: { title: "Content Groups", description: "Create collections people can enable or disable. Media and messages with any of a group’s tags belong to that collection." },
    captions: { title: "Captions", description: "Short messages shown alongside popup media. Tagged captions are only used with media carrying a matching tag." },
    prompts: { title: "Prompts", description: "Questions that ask the user for a typed response." },
    notifications: { title: "Notifications", description: "Messages displayed as desktop notifications." },
    subliminals: { title: "Subliminals", description: "Brief text overlays flashed during a session." },
    web_links: { title: "Web Links", description: "Links the experience may open in the user’s browser." },
    wallpaper: { title: "Wallpaper & Splash", description: "Choose which tagged media can be used as wallpaper or as the startup image." },
  };

  let activeTab = $state<Tab>("groups");
  let narrowWindow = $state(false);

  onMount(() => {
    const query = window.matchMedia("(max-width: 700px)");
    const update = () => (narrowWindow = query.matches);
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  });

  onMount(async () => {
    if (store.behaviour === null) store.behaviour = await api.getBehaviour();
    initializeBehaviourHistory(store.behaviour);
  });

  onDestroy(() => {
    flushBehaviourSave();
  });
</script>

<div class="flex-1 min-h-0 flex flex-col w-full">
  {#if store.behaviour === null}
    <p class="text-sm text-muted p-6">Loading…</p>
  {:else}
    <div class="flex-1 min-h-0 flex max-[700px]:flex-col">
      <aside class="w-48 max-[900px]:w-40 max-[700px]:w-full shrink-0 border-r max-[700px]:border-r-0 max-[700px]:border-b border-border bg-surface p-3 max-[700px]:py-0">
        <Tabs {tabs} active={activeTab} orientation={narrowWindow ? "horizontal" : "vertical"} onselect={(id) => (activeTab = id as Tab)} />
      </aside>

      <div class="flex-1 min-w-0 overflow-y-auto p-6 max-[700px]:p-4">
        <div class="mb-5 max-w-2xl">
          <h2 class="text-lg font-semibold text-text">{sectionInfo[activeTab].title}</h2>
          <p class="text-sm text-muted mt-1">{sectionInfo[activeTab].description}</p>
        </div>
        {#if activeTab === "groups"}
          <ContentGroupsEditor />
        {:else if activeTab === "captions"}
          <TextPoolEditor title="Captions" pool={store.behaviour!.content.captions} idPrefix="caption" />
        {:else if activeTab === "prompts"}
          <div class="flex flex-col gap-3">
            <TextPoolEditor title="Prompts" pool={store.behaviour!.content.prompts} idPrefix="prompt" />
            <label class="flex flex-col gap-1"><span class="text-xs text-muted font-medium">Submit button label</span><input bind:value={store.behaviour!.content.prompt_settings.submit_label} oninput={scheduleBehaviourSave} placeholder="Submit" class="px-2 py-1.5 rounded border border-border bg-surface text-text text-sm w-48 focus:border-accent" /></label>
          </div>
        {:else if activeTab === "notifications"}
          <TextPoolEditor title="Notifications" pool={store.behaviour!.content.notifications} idPrefix="notification" />
        {:else if activeTab === "subliminals"}
          <TextPoolEditor title="Subliminals" pool={store.behaviour!.content.subliminals} idPrefix="subliminal" />
        {:else if activeTab === "web_links"}
          <WebLinksEditor />
        {:else if activeTab === "wallpaper"}
          <div class="flex flex-col gap-6">
            <section class="flex flex-col gap-2"><div><h3 class="text-sm font-semibold text-text">Wallpaper</h3><p class="text-xs text-muted">Tags identifying wallpaper media. Leave empty to disable engine-managed wallpaper.</p></div><TagPicker tags={store.behaviour!.content.wallpaper_tags} id="wallpaper-tags" />{#if store.behaviour!.content.wallpaper_tags.length === 0}<p class="text-xs text-muted italic">No wallpaper tags selected. Lewdware will not change the wallpaper.</p>{/if}</section>
            <section class="flex flex-col gap-2"><div><h3 class="text-sm font-semibold text-text">Splash</h3><p class="text-xs text-muted">Tags identifying a startup splash image. Leave empty to disable it.</p></div><TagPicker tags={store.behaviour!.content.splash_tags} id="splash-tags" />{#if store.behaviour!.content.splash_tags.length === 0}<p class="text-xs text-muted italic">No splash tags selected. No startup image will be shown.</p>{/if}</section>
          </div>
        {/if}
      </div>
    </div>
  {/if}
</div>
