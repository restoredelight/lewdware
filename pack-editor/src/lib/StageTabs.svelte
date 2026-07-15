<script lang="ts">
  import { ArrowDown, ArrowUp, Bars3, DocumentDuplicate, EllipsisVertical, Icon, Trash } from "svelte-hero-icons";
  import Popover from "$ui/Popover.svelte";
  import type { Stage, Transition } from "./types.js";

  type Props = {
    stages: Stage[]; transitions: Transition[]; active: string; onselect: (id: string) => void;
    onmove: (from: number, to: number) => void; onduplicate: (index: number) => void;
    ondelete: (stage: Stage) => void;
  };
  let { stages, transitions, active, onselect, onmove, onduplicate, ondelete }: Props = $props();
  let dragging = $state<number | null>(null);
  let over = $state<number | null>(null);
  let tablist: HTMLDivElement;
  let dropCentres: number[] = [];
  let horizontalDrag = false;
  let settling = $state(false);

  function finishDrag() {
    dragging = null; over = null; dropCentres = [];
    settling = true;
    requestAnimationFrame(() => requestAnimationFrame(() => { settling = false; }));
    document.documentElement.classList.remove("stage-reordering");
  }

  function drop(index: number) {
    if (dragging !== null && dragging !== index) onmove(dragging, index);
    finishDrag();
  }

  function startDrag(event: PointerEvent, index: number) {
    if (event.button !== 0) return;
    event.preventDefault();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    horizontalDrag = getComputedStyle(tablist).flexDirection === "row";
    dropCentres = [...tablist.querySelectorAll<HTMLElement>(":scope > .stage-item")].map((item) => {
      const bounds = item.getBoundingClientRect();
      return horizontalDrag ? bounds.left + bounds.width / 2 : bounds.top + bounds.height / 2;
    });
    dragging = index;
    over = index;
    document.documentElement.classList.add("stage-reordering");
  }

  function updateDropTarget(event: PointerEvent) {
    if (dragging === null) return;
    event.preventDefault();
    const pointer = horizontalDrag ? event.clientX : event.clientY;
    let closest = dragging;
    let closestDistance = Infinity;
    for (const [index, centre] of dropCentres.entries()) {
      // These viewport coordinates were captured before the preview transforms were applied, so
      // animated rows cannot move their own hit targets and make the result oscillate.
      const distance = Math.abs(pointer - centre);
      if (distance < closestDistance) { closest = index; closestDistance = distance; }
    }
    over = closest;
  }

  function previewOffset(index: number): number {
    if (dragging === null || over === null || dragging === over) return 0;
    if (index === dragging) return dropCentres[over] - dropCentres[dragging];
    if (dragging < over && index > dragging && index <= over) return dropCentres[index - 1] - dropCentres[index];
    if (dragging > over && index >= over && index < dragging) return dropCentres[index + 1] - dropCentres[index];
    return 0;
  }

  function transitionAfter(index: number) {
    const from = stages[index];
    const to = stages[index + 1];
    return from && to ? transitions.find((item) => item.from_stage === from.id && item.to_stage === to.id) : undefined;
  }

</script>

<svelte:window
  onpointermove={updateDropTarget}
  onpointerup={() => { if (dragging !== null) drop(over ?? dragging); }}
  onpointercancel={() => { if (dragging !== null) finishDrag(); }}
/>

<div bind:this={tablist} class="stage-tabs" class:reordering={dragging !== null} class:settling role="tablist" aria-label="Experience stages" tabindex="-1">
  {#each stages as stage, index (stage.id)}
    <div role="presentation" style={`--drag-offset:${previewOffset(index)}px`} class:active={active === stage.id} class:dragging={dragging === index} class="stage-item">
      <button class="drag" aria-label={`Drag ${stage.label} to reorder`} onpointerdown={(event) => startDrag(event, index)}><Icon src={Bars3} mini /></button>
      <button class="stage-tab" role="tab" aria-selected={active === stage.id} tabindex={active === stage.id ? 0 : -1} onclick={() => onselect(stage.id)}>{stage.label || `Stage ${index + 1}`}</button>
      <Popover align="end" width="compact" label={`Actions for ${stage.label}`}>
        {#snippet trigger(toggle, open)}<button class="menu-trigger" onclick={toggle} aria-label={`Actions for ${stage.label}`} aria-haspopup="menu" aria-expanded={open}><Icon src={EllipsisVertical} mini /></button>{/snippet}
        {#snippet children(close)}<div class="menu">
          <button class="menu-item" role="menuitem" disabled={index === 0} onclick={() => { close(); onmove(index, index - 1); }}><Icon src={ArrowUp} mini /> Move up</button>
          <button class="menu-item" role="menuitem" disabled={index === stages.length - 1} onclick={() => { close(); onmove(index, index + 1); }}><Icon src={ArrowDown} mini /> Move down</button>
          <button class="menu-item" role="menuitem" onclick={() => { close(); onduplicate(index); }}><Icon src={DocumentDuplicate} mini /> Duplicate</button>
          <div class="separator"></div>
          <button role="menuitem" class="menu-item delete" disabled={stages.length === 1} onclick={() => { close(); ondelete(stage); }}><Icon src={Trash} mini /> Delete</button>
        </div>{/snippet}
      </Popover>
    </div>
    {#if transitionAfter(index)}
      {@const transition = transitionAfter(index)!}
      <div class="transition-item" class:active={active === transition.id} class:hidden={dragging !== null} role="presentation">
        <button class="transition-tab" role="tab" aria-selected={active === transition.id} tabindex={active === transition.id ? 0 : -1} title={transition.duration_seconds === 0 ? "Immediate transition" : `Transition over ${transition.duration_seconds} seconds`} onclick={() => onselect(transition.id)}>
          Transition
        </button>
      </div>
    {/if}
  {/each}
</div>

<style>
  :global(html.stage-reordering),:global(html.stage-reordering *){cursor:grabbing!important}
  .stage-tabs{position:relative;display:flex;min-width:0;flex-direction:column;gap:2px}.stage-tabs::before{position:absolute;z-index:0;top:18px;bottom:18px;left:50%;width:1px;background:var(--ui-border-strong);content:""}.stage-item{position:relative;z-index:1;display:flex;min-width:0;min-height:36px;align-items:center;border-radius:5px;background:var(--ui-surface);color:var(--ui-text);transition:transform 120ms ease}.stage-item:has(.menu-trigger[aria-expanded="true"]){z-index:2}.stage-item:hover{background:var(--ui-surface-raised)}.stage-item.active{background:var(--ui-accent);color:white}.stage-tabs.reordering .stage-item{transform:translateY(var(--drag-offset));pointer-events:none}.stage-tabs.reordering .stage-item:not(.dragging):not(.active):hover,.stage-tabs.settling .stage-item:not(.active):hover{background:var(--ui-surface)}.stage-tabs.settling .stage-item{transition:none}.stage-item.dragging{z-index:2;background:var(--ui-surface-raised);opacity:.7}.stage-item.active.dragging{background:var(--ui-accent)}.drag{display:grid;width:26px;height:30px;margin-left:2px;flex:none;padding:0;touch-action:none;place-items:center;border:0;background:transparent;color:currentColor;opacity:.5;cursor:grab}.drag:active{cursor:grabbing}.drag :global(svg){width:15px;height:15px}.stage-tab{min-width:0;flex:1;padding:8px 2px;overflow:hidden;border:0;background:transparent;color:inherit;font:inherit;font-size:13px;text-align:left;text-overflow:ellipsis;white-space:nowrap;cursor:pointer}.stage-tab:focus-visible,.menu-trigger:focus-visible,.drag:focus-visible,.transition-tab:focus-visible{outline:2px solid var(--ui-focus);outline-offset:-2px}.menu-trigger{display:grid;width:30px;height:30px;flex:none;padding:0;place-items:center;border:0;border-radius:4px;background:transparent;color:inherit;cursor:pointer}.menu-trigger:hover{background:rgb(255 255 255/.1)}.transition-item{position:relative;z-index:1;display:flex;min-height:46px;align-items:center;justify-content:center;color:var(--ui-muted);transition:opacity 80ms}.transition-item.hidden{opacity:0;pointer-events:none}.transition-tab{display:block;min-width:0;padding:5px 10px;border:1px solid var(--ui-border);border-radius:999px;background:var(--ui-surface);color:inherit;font:inherit;font-size:11px;text-align:center;cursor:pointer}.transition-tab:hover{border-color:var(--ui-border-strong);background:var(--ui-surface-raised);color:var(--ui-text)}.transition-item.active .transition-tab{border-color:var(--ui-accent);background:var(--ui-accent);color:white}.menu{width:100%;box-sizing:border-box;padding:4px}.menu-item{display:flex;width:100%;min-height:32px;box-sizing:border-box;padding:6px 8px;align-items:center;gap:8px;border:0;border-radius:4px;background:transparent;color:var(--ui-text);font:inherit;font-size:12px;text-align:left;cursor:pointer}.menu-item:hover:not(:disabled){background:var(--ui-surface-raised)}.menu-item:disabled{opacity:.4;cursor:not-allowed}.menu-item :global(svg){width:15px;height:15px}.menu-item.delete{color:var(--ui-danger)}.separator{height:1px;margin:4px;background:var(--ui-border)}
  @media(max-width:700px){.stage-tabs{flex-direction:row;overflow-x:auto}.stage-tabs::before{top:50%;right:75px;bottom:auto;left:75px;width:auto;height:1px}.stage-item{min-width:150px;transform:none;flex:none}.transition-item{min-width:108px}.transition-tab{text-align:center}}
</style>
