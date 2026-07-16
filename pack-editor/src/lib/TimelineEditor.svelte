<script lang="ts">
  import { Icon, Plus } from "svelte-hero-icons";
  import Button from "$ui/Button.svelte";
  import Dialog from "$ui/Dialog.svelte";
  import Select from "$ui/Select.svelte";
  import Toggle from "$ui/Toggle.svelte";
  import EventScheduleEditor from "./EventScheduleEditor.svelte";
  import StageTabs from "./StageTabs.svelte";
  import TagPicker from "./TagPicker.svelte";
  import TransitionEditor from "./TransitionEditor.svelte";
  import { scheduleBehaviourSave } from "./behaviourSave.svelte.js";
  import { store } from "./store.svelte.js";
  import type { EventSchedule, Stage } from "./types.js";
  import { duplicateStage as duplicateTimelineStage, moveStage as moveTimelineStage, normalizeTimeline, removeStage as removeTimelineStage } from "./timelineModel.js";

  const stages = $derived(store.behaviour!.experience!.timeline.stages);
  const transitions = $derived(store.behaviour!.experience!.timeline.transitions);
  let activeId = $state(""); let removing = $state<Stage|null>(null);
  let mainEl = $state<HTMLElement>();
  // WebKitGTK doesn't reliably clamp scrollTop when the panel's content shrinks,
  // leaving a shorter stage blank and unscrollable.
  $effect(() => { activeId; mainEl?.scrollTo(0, 0); });
  $effect(() => { if ((!activeId || ![...stages, ...transitions].some((item) => item.id === activeId)) && stages[0]) activeId = stages[0].id; });
  const activeIndex = $derived(stages.findIndex((stage)=>stage.id===activeId));
  const stage = $derived(activeIndex >= 0 ? stages[activeIndex] : undefined);
  const transition = $derived(transitions.find((item) => item.id === activeId));
  const transitionFrom = $derived(transition ? stages.find((item) => item.id === transition.from_stage) : undefined);
  const transitionTo = $derived(transition ? stages.find((item) => item.id === transition.to_stage) : undefined);
  const previous = $derived(activeIndex > 0 ? stages[activeIndex-1] : undefined);
  const next = $derived(activeIndex >= 0 ? stages[activeIndex + 1] : undefined);
  const outgoing = $derived(stage && next ? transitions.find((item) => item.from_stage === stage.id && item.to_stage === next.id) : undefined);
  const eventDefs = [{key:"popup",label:"Popups",interval:30},{key:"web",label:"Web links",interval:300},{key:"notification",label:"Notifications",interval:300},{key:"prompt",label:"Prompts",interval:90},{key:"subliminal",label:"Subliminals",interval:60}] as const;
  const clone=<T,>(value:T):T=>structuredClone($state.snapshot(value)) as T;
  function id(prefix:string){return `${prefix}-${crypto.randomUUID()}`}
  function changed(){normalizeTimeline(store.behaviour!.experience!.timeline);scheduleBehaviourSave()}
  function addStage(){
    // With a transition selected, insert between its two stages; otherwise after the active stage.
    const source = stage ?? transitionFrom ?? stages[stages.length - 1];
    let insertIndex = stages.length;
    if (stage) insertIndex = activeIndex + 1;
    else if (transition) {
      const toIndex = stages.findIndex((item) => item.id === transition.to_stage);
      if (toIndex >= 0) insertIndex = toIndex;
    }
    const next: Stage = source ? clone(source) : { id: "", label: "", content: {}, events: {} };
    next.id = id("stage");
    next.label = `Stage ${stages.length + 1}`;
    stages.splice(insertIndex, 0, next);
    activeId = next.id;
    changed();
  }
  function duplicate(index=activeIndex){const source=stages[index];if(!source)return;const snapshot=$state.snapshot(source) as Stage;const next=duplicateTimelineStage(store.behaviour!.experience!.timeline,index,snapshot);activeId=next.id;scheduleBehaviourSave()}
  function move(from:number,to:number){const selected=stages[from];moveTimelineStage(store.behaviour!.experience!.timeline,from,to-from);activeId=selected.id;scheduleBehaviourSave()}
  function confirmRemove(){if(!removing||stages.length===1)return;const i=stages.indexOf(removing);removeTimelineStage(store.behaviour!.experience!.timeline,removing);activeId=stages[Math.min(i,stages.length-1)].id;removing=null;scheduleBehaviourSave()}
  function eventValue(key:keyof Stage["events"]):EventSchedule|undefined{return stage?.events[key]}
  function setEvent(key:keyof Stage["events"],value?:EventSchedule){if(!stage)return;if(value)stage.events[key]=value;else delete stage.events[key];scheduleBehaviourSave()}
  function transitionSummary() {
    if (!outgoing || outgoing.duration_seconds === 0) return "Immediately";
    const minutes = outgoing.duration_seconds / 60;
    return `Gradually over ${Number.isInteger(minutes) ? `${minutes} minute${minutes === 1 ? "" : "s"}` : `${outgoing.duration_seconds} seconds`}`;
  }
</script>
<section class="layout">
  <aside><div class="tabs"><StageTabs {stages} {transitions} active={activeId} onselect={(id)=>activeId=id} onmove={move} onduplicate={duplicate} ondelete={(item)=>removing=item}/></div><Button size="compact" class="w-full max-[700px]:w-auto max-[700px]:shrink-0" onclick={addStage}><Icon src={Plus} mini/> Add stage</Button></aside>
  {#if stage}<main bind:this={mainEl}>
    <div class="panel">
    <header><div><input class="stage-name" aria-label="Stage name" bind:value={stage.label} oninput={scheduleBehaviourSave}/><p>{activeIndex===0?"Active when the session begins.":"A complete set of behaviour for this part of the session."}</p></div></header>

    <section class="card"><div class="section-title"><div><h3>Content selection</h3><p>Choose which tagged content and wallpaper are eligible during this stage.</p></div></div>
      <div class="toggle-row"><div><strong>Limit content by tag</strong>{#if previous}<small>Previous stage: {previous.content.tags?`${previous.content.tags.length} selected`:"All content"}</small>{/if}</div><Toggle ariaLabel="Limit content by tag" checked={!!stage.content.tags} onchange={(on)=>{if(on)stage.content.tags=[];else delete stage.content.tags;scheduleBehaviourSave()}}/></div>{#if stage.content.tags}<TagPicker tags={stage.content.tags} id={`stage-content-${stage.id}`} onchange={(tags)=>(stage.content.tags=tags)}/>{/if}
      <div class="toggle-row"><div><strong>Override wallpaper selection</strong>{#if previous}<small>Previous stage: {previous.content.wallpaper_tags?`${previous.content.wallpaper_tags.length} selected`:"Pack default"}</small>{/if}</div><Toggle ariaLabel="Override wallpaper selection" checked={!!stage.content.wallpaper_tags} onchange={(on)=>{if(on)stage.content.wallpaper_tags=[];else delete stage.content.wallpaper_tags;scheduleBehaviourSave()}}/></div>{#if stage.content.wallpaper_tags}<TagPicker tags={stage.content.wallpaper_tags} id={`stage-wallpaper-${stage.id}`} onchange={(tags)=>(stage.content.wallpaper_tags=tags)}/>{/if}
    </section>

    <section class="card"><div class="section-title"><div><h3>Events</h3><p>Enable events and choose how frequently they spawn.</p></div></div>{#each eventDefs as def}<EventScheduleEditor label={def.label} value={eventValue(def.key)} previous={previous?.events[def.key]} defaultInterval={def.interval} onchange={(value)=>setEvent(def.key,value)}/>{/each}</section>

    <section class="card"><div class="section-title"><div><h3>Movement</h3><p>Control how quickly popup media moves.</p></div><Toggle ariaLabel="Enable movement" checked={!!stage.movement} onchange={(on)=>{if(on)stage.movement={minimum_speed:50,maximum_speed:150};else delete stage.movement;scheduleBehaviourSave()}}/></div>{#if stage.movement}<div class="fields"><label>Minimum speed<input type="number" bind:value={stage.movement.minimum_speed} oninput={scheduleBehaviourSave}/><small>Previous: {previous?.movement?.minimum_speed??"Off"}</small></label><label>Maximum speed<input type="number" bind:value={stage.movement.maximum_speed} oninput={scheduleBehaviourSave}/><small>Previous: {previous?.movement?.maximum_speed??"Off"}</small></label></div>{/if}</section>

    <section class="card"><div class="section-title"><div><h3>Mitosis</h3><p>Allow popup media to create additional copies.</p></div><Toggle ariaLabel="Enable mitosis" checked={!!stage.mitosis} onchange={(on)=>{if(on)stage.mitosis={chance:.5,count:2};else delete stage.mitosis;scheduleBehaviourSave()}}/></div>{#if stage.mitosis}<div class="fields"><label>Chance (0–1)<input type="number" min="0" max="1" step=".05" bind:value={stage.mitosis.chance} oninput={scheduleBehaviourSave}/><small>Previous: {previous?.mitosis?.chance??"Off"}</small></label><label>Copies<input type="number" min="1" step="1" bind:value={stage.mitosis.count} oninput={scheduleBehaviourSave}/><small>Previous: {previous?.mitosis?.count??"Off"}</small></label></div>{/if}</section>

    <section class="card"><div class="section-title"><div><h3>Stage duration</h3><p>{activeIndex===stages.length-1?"The final stage continues until the session ends.":"Choose how long these settings stay fully active."}</p></div></div>{#if stage.end}<div class="fields"><label>Keep these settings for (minutes)<input type="number" min="0" value={(stage.end.duration_seconds??300)/60} oninput={(e)=>{stage.end!.duration_seconds=e.currentTarget.valueAsNumber*60;scheduleBehaviourSave()}}/>{#if stage.end.duration_seconds===0}<small>The transition begins as soon as this stage is reached.</small>{/if}</label><Select label="Additional condition" value={stage.end.event_count?stage.end.event_count.event:"none"} options={[{value:"none",label:"No event condition"},...eventDefs.map(d=>({value:d.key,label:`${d.label} spawned`}))]} onchange={(v)=>{if(v==="none")delete stage.end!.event_count;else stage.end!.event_count={event:v as any,count:10,scope:"stage"};scheduleBehaviourSave()}}/>{#if stage.end.event_count}<label>Event count<input type="number" min="1" bind:value={stage.end.event_count.count} oninput={scheduleBehaviourSave}/></label><Select label="Advance when" value={stage.end.strategy} options={[{value:"any",label:"Either condition is reached"},{value:"all",label:"Both conditions are reached"}]} onchange={(v)=>{stage.end!.strategy=v as "any"|"all";scheduleBehaviourSave()}}/>{/if}</div>{#if next && outgoing}<div class="next-summary"><span>Then change to <button onclick={()=>activeId=next.id}>{next.label}</button></span><button class="transition-link" onclick={()=>activeId=outgoing.id}>{transitionSummary()} <span aria-hidden="true">→</span></button></div>{/if}{/if}</section>
    </div>
  </main>{:else if transition && transitionFrom && transitionTo}<TransitionEditor transitionId={transition.id} from={transitionFrom} to={transitionTo} onstage={(id)=>activeId=id}/>{/if}
</section>
{#if removing}<Dialog title={`Remove “${removing.label}”?`} description="This stage and its settings will be removed. Transitions to its neighbours will be reset." buttons={[{label:"Cancel",onclick:()=>removing=null},{label:"Remove stage",destructive:true,onclick:confirmRemove}]} onclose={()=>removing=null}/>{/if}
<style>
  .layout{display:flex;min-height:0;flex:1}.layout>aside{display:flex;width:192px;flex:none;padding:12px;flex-direction:column;gap:12px;border-right:1px solid var(--ui-border);background:var(--ui-surface)}.tabs{min-height:0;flex:1;overflow-y:auto;scrollbar-gutter:stable;margin-right:-12px;padding-right:8px}main{flex:1;min-width:0;padding:24px;overflow-y:auto}main .panel{display:flex;width:100%;max-width:800px;min-width:0;margin-inline:auto;flex-direction:column;gap:14px}.panel>header{display:flex;align-items:start;justify-content:space-between;gap:16px}.stage-name{width:100%;padding:0;border:0;background:transparent;color:var(--ui-text);font-size:17px;font-weight:650}.stage-name:focus-visible{outline:2px solid var(--ui-focus);outline-offset:3px}header p,.section-title p{margin:4px 0 0;color:var(--ui-muted);font-size:12px}.card{display:flex;padding:16px;flex-direction:column;gap:12px;border:1px solid var(--ui-border);border-radius:var(--ui-radius-md);background:var(--ui-surface)}.section-title,.toggle-row{display:flex;align-items:center;justify-content:space-between;gap:16px}.section-title h3{margin:0;font-size:16px}.toggle-row{padding-top:10px;border-top:1px solid var(--ui-border)}.toggle-row strong,.toggle-row small{display:block;font-size:12px}.toggle-row small{margin-top:2px;color:var(--ui-muted);font-size:10px}.fields{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));align-items:start;gap:12px}.fields label{display:flex;min-width:0;flex-direction:column;gap:5px;color:var(--ui-text);font-size:12px;font-weight:600}.fields input{width:100%;height:36px;padding:0 9px;border:1px solid var(--ui-border);border-radius:var(--ui-radius-sm);background:var(--ui-bg);color:var(--ui-text);font:inherit;font-weight:400;font-size:13px}.fields small{color:var(--ui-muted);font-size:10px;font-weight:400}.next-summary{display:flex;padding-top:12px;align-items:center;justify-content:space-between;gap:12px;border-top:1px solid var(--ui-border);color:var(--ui-muted);font-size:12px}.next-summary button{padding:0;border:0;background:transparent;color:var(--ui-text);font:inherit;text-decoration:underline;text-decoration-color:var(--ui-border-strong);text-underline-offset:3px;cursor:pointer}.next-summary .transition-link{padding:7px 9px;border-radius:var(--ui-radius-sm);background:var(--ui-bg);text-decoration:none}.transition-link:hover{background:var(--ui-surface-raised)}@media(max-width:700px){.layout{flex-direction:column}.layout>aside{width:100%;padding-block:0;align-items:center;flex-direction:row;border-right:0;border-bottom:1px solid var(--ui-border)}.tabs{overflow-x:auto;margin-right:0;padding-right:0}main{padding:16px}.fields{grid-template-columns:1fr}.next-summary{align-items:flex-start;flex-direction:column}}
</style>
