<!--
  A live miniature of one window theme: its border, title bar and close button, and a dialog's
  panel, text field and buttons.

  Everything is drawn from `ThemeLook` — the same values `shared::theme` hands the engine — so this
  is a second *renderer* of one description rather than a second description. It is interactive on
  purpose: hover the close button, press a button, click into the field. Per-state chrome is
  exactly what a static thumbnail cannot show, and it is much of what tells two themes apart.

  Geometry follows the engine's, not CSS habit, because the two are visibly different:

    - A title-bar button is the **full height of the bar** and `width_ratio × that height` wide.
      `inset` is a horizontal offset from the bar's end only, and there is one of it, not two
      (`Header::button_span`/`buttons_extent` in `lewdware/src/window/header.rs`).
    - A glyph is sized from its *button*, not the bar: `extent / 6` from the centre on each axis,
      pulled in by 1/√2 again inside a circle (`Header::glyph_reach`).
    - The dialog is a 10pt margin, centred content, and a centred button row
      (`paint_dialog` in `window/layer/egui.rs`).

  Drawn at true logical size (`scale = 1`), so a 14pt UI font is 14px here as well — scaling the
  window up without scaling the type is what made an earlier version look nothing like the real
  thing.

  Two deliberate limits, both matching what the engine can actually do: corners stay square, since
  its CPU path cannot draw transparent corner pixels; and Aqua's inert traffic lights never respond
  to the pointer.
-->
<script lang="ts">
	import './theme-fonts.css';
	import type {
		BorderRing,
		ChromeButton,
		FaceName,
		Fill,
		Stroke,
		ThemeLook,
		Widgets
	} from './types';

	type Props = {
		look: ThemeLook;
		/** The window title, which is what the theme's title alignment is demonstrated on. */
		title?: string;
		/** Content width in logical pixels, before the border is added around it. */
		width?: number;
	};

	let { look, title = 'Lewdware', width = 236 }: Props = $props();

	const chrome = $derived(look.chrome);
	const widgets = $derived(look.widgets);
	const headerHeight = $derived(look.metrics.header_height);

	/** The dialog's own margin, and the space `paint_dialog` puts after every element. */
	const DIALOG_MARGIN = 10;

	function fill(f: Fill): string {
		if ('Solid' in f) return f.Solid;
		if ('VerticalGradient' in f) {
			return `linear-gradient(to bottom, ${f.VerticalGradient.from}, ${f.VerticalGradient.to})`;
		}
		// One stripe line every `period` pixels — Mac OS 9's pinstriped bar.
		const { base, stripe, period } = f.Pinstripe;
		return `repeating-linear-gradient(to bottom, ${base} 0 ${period - 1}px, ${stripe} ${period - 1}px ${period}px)`;
	}

	/** A ring as the four border colours of one nested box. */
	function ringColors(ring: BorderRing): string {
		if ('Uniform' in ring) return ring.Uniform;
		const { top_left, bottom_right } = ring.Bevel;
		return `${top_left} ${bottom_right} ${bottom_right} ${top_left}`;
	}

	const ringStyle = (ring: BorderRing) => `border:1px solid;border-color:${ringColors(ring)};`;

	/// The bundled file for a face. Every face a theme can name has one, so nothing here falls back
	/// to a system font that would differ from what the engine draws — the generic tails are a
	/// last resort for a face this app has not been taught about, which the test in `shared` makes
	/// hard to reach.
	function faceStack(face: FaceName): string {
		switch (face) {
			case 'default':
				return "'lw-default', sans-serif";
			case 'pixel':
				return "'lw-pixel', monospace";
			case 'selawik':
				return "'lw-selawik', sans-serif";
			case 'cantarell':
				return "'lw-cantarell', sans-serif";
			case 'inter':
				return "'lw-inter', sans-serif";
			case 'noto-sans':
				return "'lw-noto-sans', sans-serif";
			case 'liberation-sans':
				return "'lw-liberation', sans-serif";
			case 'liberation-sans-bold':
				return "'lw-liberation-bold', sans-serif";
			case 'source-sans':
				return "'lw-source-sans', sans-serif";
			case 'source-sans-semibold':
				return "'lw-source-sans-semibold', sans-serif";
			// `mono` and `display` are text-popup faces no theme can name, so they are not
			// bundled here — see theme-fonts.css.
			default:
				return 'sans-serif';
		}
	}

	const buttonWidth = (button: ChromeButton) => button.width_ratio * headerHeight;

	/** `Header::buttons_extent`: the whole cluster, including its single inset and any gaps. */
	const clusterWidth = $derived(
		chrome.buttons.buttons.length === 0
			? 0
			: chrome.buttons.inset +
					chrome.buttons.buttons.reduce((total, b) => total + buttonWidth(b), 0) +
					chrome.buttons.gap * (chrome.buttons.buttons.length - 1)
	);

	/** `Header::glyph_reach`: how far the mark reaches from its button's centre, on each axis. */
	function glyphSize(button: ChromeButton): number {
		const extent = Math.min(buttonWidth(button), headerHeight);
		if (button.glyph === 'None') return 0;
		const wide = button.glyph === 'WideCross';
		if (button.shape === 'Circle') {
			return 2 * ((extent / (wide ? 3 : 6)) * Math.SQRT1_2);
		}
		return 2 * (extent / 6);
	}

	const strokeCss = (s: Stroke) =>
		s.width === 0 ? '0 solid transparent' : `${s.width}px solid ${s.color}`;

	const bevel = $derived(widgets.edge === 'Flat' ? null : widgets.edge.Bevel);

	const defaultFilled = $derived(
		'Filled' in widgets.default_button ? widgets.default_button.Filled : null
	);
	const defaultOutline = $derived(
		'Outline' in widgets.default_button ? widgets.default_button.Outline : null
	);

	// The field is a `contenteditable` div rather than an `<input>`, for one reason: WebKit renders
	// an input's text inside a UA shadow root (itself a `contenteditable="plaintext-only"` div),
	// and `::selection` on the host does not cross into a shadow tree. The theme's highlight was
	// therefore unstylable, and every card fell back to the browser's own selection — convincing
	// enough on a focused card, and invisible on an unfocused one, since WebKit draws an inactive
	// selection in a grey that `plain`'s near-black field swallows whole.
	//
	// The colours also have to reach the rule as literals rather than through `var()`, which
	// WebKit does not resolve inside `::selection`.
	//
	// Kept to the two properties `::selection` is actually specified to take. An earlier attempt
	// also set `-webkit-text-fill-color`, which takes precedence over `color` and rendered the
	// selected glyphs as nothing at all.
	//
	// And the background is passed at alpha 254/255 rather than opaque. WebKit runs an *opaque*
	// selection background through `blendWithWhite()`, which looks for a lighter colour that would
	// match the request when composited over white — there is no such colour for a dark one, so
	// every dark palette's highlight came out pitch black while `#ffffff` sailed through. The
	// transform is skipped for a colour that already carries alpha, and one step down from opaque
	// is invisible to the eye.
	// Scoped to the whole card, not just the field: egui ships `selectable_labels: true`, so a
	// dialog's own text is selectable in the engine and takes the same colours.
	const uid = $props.id();
	const cardClass = $derived(`lw-card-${uid}`);

	/** Guards the interpolation below. Every colour comes from our own catalogue as `#rrggbb`. */
	const isColor = (value: string) => /^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$/.test(value);

	/** `#rrggbb` -> `#rrggbbfe`; a colour that already states its alpha is left alone. */
	const nearlyOpaque = (color: string) => (color.length === 7 ? `${color}fe` : color);

	const selectionCss = $derived(
		isColor(widgets.selection) && isColor(widgets.selection_text)
			? `.${cardClass} ::selection{background-color:${nearlyOpaque(
					widgets.selection
				)};color:${widgets.selection_text};}`
			: ''
	);

	/** What `paint_dialog` pads a text field by to reach the theme's control height. */
	const fieldPadding = $derived.by(() => {
		const row = widgets.font_size * 1.25;
		const vertical = Math.max(2, Math.round((widgets.metrics.control_height - row) / 2));
		return `${vertical}px ${widgets.metrics.button_padding[0]}px`;
	});
</script>

{#snippet glyphMark(button: ChromeButton, color: string)}
	{#if button.glyph !== 'None'}
		{@const size = glyphSize(button)}
		<svg
			viewBox="0 0 10 10"
			aria-hidden="true"
			style="width:{size}px;height:{size}px;overflow:visible"
		>
			<path
				d="M0 0 L10 10 M10 0 L0 10"
				stroke={color}
				stroke-width={button.glyph === 'WideCross' ? 1.1 : 1.4}
				stroke-linecap="square"
			/>
		</svg>
	{/if}
{/snippet}

<!-- Border rings, outermost first, as nested boxes: a bevel needs a different colour per side,
     which an inset box-shadow cannot express. -->
{#snippet ring(rings: BorderRing[], index: number)}
	{#if index < rings.length}
		<div style={ringStyle(rings[index])}>
			{@render ring(rings, index + 1)}
		</div>
	{:else}
		{@render windowBody()}
	{/if}
{/snippet}

{#snippet windowBody()}
	<div
		class="header"
		style="height:{headerHeight}px;background:{fill(chrome.header)};flex-direction:{chrome.buttons
			.side === 'Left'
			? 'row'
			: 'row-reverse'};padding-inline-{chrome.buttons.side === 'Left' ? 'start' : 'end'}:{chrome
			.buttons.inset}px;gap:{chrome.buttons.gap}px"
	>
		{#each chrome.buttons.buttons as button, i (i)}
			<span
				class="chrome-button"
				class:inert={button.action === 'Inert'}
				style="width:{buttonWidth(button)}px;--idle:{fill(button.idle.fill)};--hover:{fill(
					button.hover.fill
				)};--active:{fill(button.active.fill)};--idle-glyph:{button.idle
					.glyph};--hover-glyph:{button.hover.glyph};--rim:{button.idle.rim ??
					'transparent'};{button.shape === 'Circle'
					? // A circle is `min(width, height)` across and centred in the bar, not stretched to
						// its full height — `Header::draw_buttons`.
						`border-radius:50%;align-self:center;height:${Math.min(buttonWidth(button), headerHeight)}px`
					: ''}"
				title={button.action === 'Close' ? 'Close' : 'Inert — drawn for the look, does nothing'}
			>
				{@render glyphMark(button, 'currentColor')}
			</span>
		{/each}

		<span
			class="title"
			style="font-family:{faceStack(chrome.title.font)};font-size:{chrome.title
				.size}px;color:{chrome.title.color};text-align:{chrome.title.align};padding-inline:{chrome
				.title.padding}px;{chrome.title.align === 'center'
				? // The engine centres a title across the *whole* bar when it fits, not in what the
					// buttons leave over. Matching that means balancing the cluster on the far side.
					`margin-inline-${chrome.buttons.side === 'Left' ? 'end' : 'start'}:${clusterWidth}px`
				: ''}"
		>
			{title}
		</span>
	</div>

	<div
		class="body"
		style="background:{widgets.panel};color:{widgets.text};font-family:{faceStack(
			widgets.font
		)};font-size:{widgets.font_size}px;padding:{DIALOG_MARGIN}px;gap:{widgets.metrics
			.item_spacing[1] + DIALOG_MARGIN}px"
	>
		<span>Are you sure?</span>

		{#if bevel}
			<!-- A field is permanently recessed under a bevelled theme, exactly as `bevel::input_edge`
			     paints it: the same inverted rings a held button uses. -->
			<div class="field-box" style="{ringStyle(bevel.pressed[0])}background:{widgets.field}">
				{@render field()}
			</div>
		{:else}
			<div
				class="field-box"
				style="border:{strokeCss(
					widgets.idle.border
				)};background:{widgets.field};border-radius:{widgets.metrics.corner_radius}px"
			>
				{@render field()}
			</div>
		{/if}

		<div class="buttons" style="gap:{widgets.metrics.item_spacing[0]}px">
			{@render dialogButton('Cancel', false)}
			{@render dialogButton('OK', true)}
		</div>
	</div>
{/snippet}

{#snippet field()}
	<!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
	<div
		class="field"
		contenteditable="plaintext-only"
		role="textbox"
		tabindex="0"
		aria-label="Example text field"
		spellcheck="false"
		onkeydown={(event) => {
			// Single-line, like the `TextEdit` this is a picture of.
			if (event.key === 'Enter') event.preventDefault();
		}}
		style="color:{widgets.text};caret-color:{widgets.caret.color};padding:{fieldPadding}"
	>
		Yes
	</div>
{/snippet}

{#snippet dialogButton(label: string, isDefault: boolean)}
	{@const filled = isDefault ? defaultFilled : null}
	<button
		class="dialog-button"
		class:bevelled={!!bevel}
		type="button"
		style="--idle:{filled?.idle ?? widgets.idle.fill};--hover:{filled?.hover ??
			widgets.hover.fill};--active:{filled?.active ?? widgets.pressed.fill};color:{filled?.text ??
			widgets.text};{bevel
			? `border:1px solid;border-color:${ringColors(bevel.raised[0])}`
			: `border:${filled ? strokeCss(filled.border) : strokeCss(widgets.idle.border)}`};border-radius:{widgets
			.metrics.corner_radius}px;padding:{widgets.metrics.button_padding[1]}px {widgets.metrics
			.button_padding[0]}px;min-height:{widgets.metrics.control_height}px;outline:{isDefault &&
		defaultOutline
			? `${defaultOutline.width}px solid ${defaultOutline.color}`
			: 'none'}"
	>
		{label}
	</button>
{/snippet}

<svelte:head>
	<!-- Literal colours rather than a scoped rule over custom properties; see `selectionCss`. -->
	{@html `<style>${selectionCss}</style>`}
</svelte:head>

<div class="frame {cardClass}" style="width:{width}px">
	{@render ring(chrome.border, 0)}
</div>

<style>
	/* Content-box throughout, like the engine's own geometry: a window is its content plus a
	   border, so a 3px Win95 bevel is three real pixels around the same content area. */
	.frame :global(div) {
		box-sizing: content-box;
	}

	.header {
		display: flex;
		align-items: stretch;
		overflow: hidden;
	}

	.title {
		flex: 1;
		min-width: 0;
		align-self: center;
		overflow: hidden;
		white-space: nowrap;
		text-overflow: ellipsis;
		line-height: 1;
	}

	/* Full bar height and `width_ratio` of it wide, as the engine draws them. */
	.chrome-button {
		display: flex;
		align-items: center;
		justify-content: center;
		flex: none;
		background: var(--idle);
		color: var(--idle-glyph);
		box-shadow: inset 0 0 0 1px var(--rim);
	}

	.chrome-button:not(.inert):hover {
		background: var(--hover);
		/* Aqua's marks only appear on hover, which is the theme's own idiom, not a state change
		   invented here — the colour comes from its `hover` paint. */
		color: var(--hover-glyph);
	}

	.chrome-button:not(.inert):active {
		background: var(--active);
	}

	/* `top_down(Align::Center)`, so text and the button row centre and the field fills. */
	.body {
		display: flex;
		flex-direction: column;
		align-items: center;
		line-height: 1.3;
	}

	.field-box {
		width: 100%;
	}

	.buttons {
		display: flex;
	}

	.field {
		width: 100%;
		border: none;
		outline: none;
		background: none;
		font: inherit;
		box-sizing: border-box;
		/* Single-line, as a `TextEdit` is. `nowrap` rather than `pre` so the markup's own
		   indentation collapses instead of being rendered inside the field. */
		white-space: nowrap;
		overflow: hidden;
		/* egui puts an I-beam over a text field too. */
		cursor: text;
	}

	/* No `cursor: pointer`: the engine leaves the pointer alone over a themed button, and every
	   era these imitate did too. Nicer-feeling, but not what it is a picture of. */
	.dialog-button {
		background: var(--idle);
		font: inherit;
		box-sizing: border-box;
	}

	.dialog-button:hover {
		background: var(--hover);
	}

	.dialog-button:active {
		background: var(--active);
	}

	/* A held Win95 button shifts its label down and right, as though the face itself had moved —
	   `bevel::button` does the same. */
	.dialog-button.bevelled:active {
		transform: translate(1px, 1px);
	}
</style>
