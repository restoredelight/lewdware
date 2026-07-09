export interface AliasVariant {
	value: string; // e.g. '"linear"'
	desc?: string; // e.g. the "Ubuntu-Light, matching..." text after `"default"` in TextFont
}

export interface Alias {
	name: string;
	desc?: string;
	type?: string; // simple type, e.g. "number | { percent: number }"
	variants?: AliasVariant[];
}

export interface Field {
	name: string;
	optional: boolean;
	type: string;
	desc?: string;
}

export interface Param {
	name: string;
	optional: boolean;
	type: string;
	desc?: string;
}

// A description as a sequence of blocks -- a blank doc-comment line starts a new paragraph, and a
// fenced ```lang ... ``` block is preserved verbatim (for syntax highlighting) instead of being
// reflowed. Consecutive non-blank lines within a paragraph are joined with spaces, since they're
// just word-wrapped for source readability, not meaningful line breaks.
export type DescBlock = { type: 'paragraph'; text: string } | { type: 'code'; lang: string; code: string };

export interface Func {
	name: string;
	fullName: string; // e.g. "lewdware.spawn_image_popup" or "Window:close"
	className?: string; // set for class methods
	sep?: '.' | ':';
	params: Param[];
	returnType?: string;
	desc: DescBlock[];
}

export interface Class {
	name: string;
	parents: string[];
	desc: DescBlock[];
	fields: Field[];
	methods: Func[];
}

export interface Namespace {
	name: string;
	functions: Func[];
}

export interface ApiDoc {
	aliases: Alias[];
	classes: Class[];
	namespaces: Namespace[];
}

// ---------------------------------------------------------------------------
// Block extraction
// ---------------------------------------------------------------------------

interface Block {
	comments: string[]; // each line stripped of leading `---` and one optional space
	codeLine?: string;
}

function getBlocks(source: string): Block[] {
	const blocks: Block[] = [];
	let comments: string[] = [];

	for (const rawLine of source.split('\n')) {
		const line = rawLine.trim();

		if (line.startsWith('---')) {
			// Strip `---` and one optional leading space
			comments.push(line.slice(3).replace(/^ /, ''));
		} else if (line === '') {
			if (comments.length > 0) {
				blocks.push({ comments: [...comments] });
				comments = [];
			}
		} else {
			if (comments.length > 0) {
				blocks.push({ comments: [...comments], codeLine: line });
				comments = [];
			}
		}
	}

	if (comments.length > 0) blocks.push({ comments });
	return blocks;
}

// Merge continuation lines into the preceding line.
// A continuation line is either:
//   - indented (starts with a space, meaning the `---` was followed by spaces), OR
//   - part of an open bracket group from a previous line (e.g. the closing `}` in
//     a multiline `---@param opts? {\n---   key: type,\n---}` block)
// Lines inside a fenced ```lang ... ``` code block are passed through verbatim and never merged
// or bracket-tracked -- indentation and braces there are code formatting, not continuation markup.
function joinContinuations(lines: string[]): string[] {
	const result: string[] = [];
	let openDepth = 0;
	let inFence = false;

	for (const line of lines) {
		if (/^```(\w*)\s*$/.test(line)) {
			inFence = !inFence;
			result.push(line);
			continue;
		}

		if (inFence) {
			result.push(line);
			continue;
		}

		if ((openDepth > 0 || line.startsWith(' ')) && result.length > 0) {
			result[result.length - 1] += ' ' + line.trim();
		} else {
			result.push(line);
		}

		for (const ch of line) {
			if (ch === '{' || ch === '[' || ch === '(') openDepth++;
			else if (ch === '}' || ch === ']' || ch === ')') openDepth = Math.max(0, openDepth - 1);
		}
	}

	return result;
}

// ---------------------------------------------------------------------------
// Type reader — reads a LuaCATS type expression from the start of a string.
// Stops at a space that is NOT followed by `|` (union) or `&` (intersection),
// respecting nesting depth of `{`, `[`, `(`.
// ---------------------------------------------------------------------------

function readType(s: string): { type: string; rest: string } {
	let i = 0;
	let depth = 0;
	let prevNonSpace = '';

	while (i < s.length) {
		const ch = s[i];
		if (ch === '{' || ch === '[' || ch === '(') {
			depth++;
			prevNonSpace = ch;
			i++;
			continue;
		}
		if (ch === '}' || ch === ']' || ch === ')') {
			depth--;
			prevNonSpace = ch;
			i++;
			continue;
		}
		if (depth === 0 && ch === ' ') {
			const ahead = s.slice(i + 1).trimStart();
			if (ahead.startsWith('|') || ahead.startsWith('&') ||
				prevNonSpace === '|' || prevNonSpace === '&') {
				i++;
				continue;
			}
			break;
		}
		if (ch !== ' ') prevNonSpace = ch;
		i++;
	}

	return { type: s.slice(0, i).trim(), rest: s.slice(i).trim() };
}

// ---------------------------------------------------------------------------
// Description blocks -- paragraphs and fenced code blocks
// ---------------------------------------------------------------------------

// `lines` is a run of doc-comment lines (blank lines included) with all `@tag` lines already
// excluded by the caller.
function parseDescriptionBlocks(lines: string[]): DescBlock[] {
	const blocks: DescBlock[] = [];
	let paragraph: string[] = [];
	let codeLines: string[] | null = null;
	let codeLang = '';

	function flushParagraph() {
		if (paragraph.length > 0) {
			blocks.push({ type: 'paragraph', text: paragraph.join(' ') });
			paragraph = [];
		}
	}

	for (const line of lines) {
		const fence = line.match(/^```(\w*)\s*$/);

		if (codeLines) {
			if (fence && !fence[1]) {
				blocks.push({ type: 'code', lang: codeLang, code: codeLines.join('\n') });
				codeLines = null;
			} else {
				codeLines.push(line);
			}
			continue;
		}

		if (fence) {
			flushParagraph();
			codeLines = [];
			// Every fenced block in this API reference is Lua; default to that rather than an
			// unregistered Shiki language if a fence is ever left unlabeled.
			codeLang = fence[1] || 'lua';
			continue;
		}

		if (line === '') {
			flushParagraph();
			continue;
		}

		paragraph.push(line);
	}

	flushParagraph();
	// An unterminated fence shouldn't happen, but flush whatever was gathered rather than
	// silently dropping it.
	if (codeLines) blocks.push({ type: 'code', lang: codeLang, code: codeLines.join('\n') });

	return blocks;
}

// ---------------------------------------------------------------------------
// Individual annotation parsers
// ---------------------------------------------------------------------------

function parseFieldLine(line: string): Field | null {
	// @field name? type [desc]  OR  @field name type [desc]
	const m = line.match(/^@field\s+(\w+)(\?)?\s*(.*)?$/);
	if (!m) return null;
	const name = m[1];
	const optional = Boolean(m[2]);
	const rest = (m[3] ?? '').trim();
	if (!rest) return { name, optional, type: 'any' };
	const { type, rest: desc } = readType(rest);
	return { name, optional, type, desc: desc || undefined };
}

function parseParamLine(line: string): Param | null {
	// @param name? type [desc]
	const m = line.match(/^@param\s+(\w+)(\?)?\s+(.*)?$/);
	if (!m) return null;
	const name = m[1];
	const optional = Boolean(m[2]);
	const rest = (m[3] ?? '').trim();
	const { type, rest: desc } = readType(rest);
	return { name, optional, type, desc: desc || undefined };
}

// ---------------------------------------------------------------------------
// Block parsers
// ---------------------------------------------------------------------------

function parseClassBlock(block: Block): Class | null {
	const lines = joinContinuations(block.comments);

	let name = '';
	// A class may extend more than one parent, e.g. `@class TextPopupOpts : PopupOpts, TextStyle`.
	let parents: string[] = [];
	const fields: Field[] = [];
	const descLines: string[] = [];
	let gatheringDesc = false;
	let found = false;

	for (const line of lines) {
		if (line.startsWith('@class ')) {
			const m = line.match(/^@class\s+(\w+)(?:\s*:\s*([\w\s,]+))?/);
			if (!m) continue;
			name = m[1];
			parents = m[2]
				? m[2].split(',').map((p) => p.trim()).filter(Boolean)
				: [];
			found = true;
			gatheringDesc = true;
			continue;
		}

		if (!found) continue;

		if (line.startsWith('@field ')) {
			gatheringDesc = false;
			const f = parseFieldLine(line);
			if (f) fields.push(f);
			continue;
		}

		if (line.startsWith('@')) {
			gatheringDesc = false;
			continue;
		}

		if (gatheringDesc) {
			descLines.push(line);
		}
	}

	if (!found) return null;
	return {
		name,
		parents,
		desc: parseDescriptionBlocks(descLines),
		fields,
		methods: [],
	};
}

function parseAliasBlock(block: Block): Alias[] {
	const lines = joinContinuations(block.comments);
	const results: Alias[] = [];

	let name = '';
	let type = '';
	let desc = '';
	let variants: AliasVariant[] = [];

	function flush() {
		if (!name) return;
		results.push({
			name,
			desc: desc || undefined,
			type: type || undefined,
			variants: variants.length > 0 ? variants : undefined,
		});
	}

	for (const line of lines) {
		if (line.startsWith('@alias ')) {
			flush();
			name = '';
			type = '';
			desc = '';
			variants = [];

			const m = line.match(/^@alias\s+(\w+)(?:\s+(.+))?$/);
			if (!m) continue;
			name = m[1];
			const rest = (m[2] ?? '').trim();
			if (rest) {
				const parsed = readType(rest);
				type = parsed.type;
				desc = parsed.rest;
			}
			continue;
		}

		if (line.startsWith('| ')) {
			// A variant is a quoted string (or bare token) literal, optionally followed by a
			// description, e.g. `| "default" Ubuntu-Light, matching the window header/chrome.`
			const rest = line.slice(2).trim();
			const m = rest.match(/^("(?:[^"\\]|\\.)*"|\S+)\s*(.*)$/);
			variants.push(
				m ? { value: m[1], desc: m[2] || undefined } : { value: rest },
			);
			continue;
		}

		if (!line.startsWith('@') && name && line !== '') {
			if (!desc) desc = line;
		}
	}

	flush();
	return results;
}

function parseFuncBlock(block: Block & { codeLine: string }): Func | null {
	const codeLine = block.codeLine;
	const lines = joinContinuations(block.comments);

	// Parse the function declaration
	let className: string | undefined;
	let funcName: string;
	let sep: '.' | ':' | undefined;
	let fullName: string;

	// Pattern 1: function ClassName:method(...)
	const methodMatch = codeLine.match(/^function\s+(\w+)(:)(\w+)\s*\(/);
	if (methodMatch) {
		className = methodMatch[1];
		sep = ':';
		funcName = methodMatch[3];
		fullName = `${className}:${funcName}`;
	} else {
		// Pattern 2: function ns1.ns2.funcName(...) — greedily matches the rightmost dot
		const nsMatch = codeLine.match(/^function\s+((?:\w+\.)*\w+)\.(\w+)\s*\(/);
		if (nsMatch) {
			sep = '.';
			funcName = nsMatch[2];
			fullName = `${nsMatch[1]}.${funcName}`;
		} else {
			return null;
		}
	}

	const params: Param[] = [];
	let returnType: string | undefined;
	const descLines: string[] = [];

	for (const line of lines) {
		if (line.startsWith('@param ')) {
			const p = parseParamLine(line);
			if (p) params.push(p);
			continue;
		}
		if (line.startsWith('@return ')) {
			returnType = line.slice('@return '.length).trim();
			continue;
		}
		if (!line.startsWith('@')) {
			descLines.push(line);
		}
	}

	return {
		name: funcName,
		fullName,
		className,
		sep,
		params,
		returnType,
		desc: parseDescriptionBlocks(descLines),
	};
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

export function parseLuaCATS(source: string): ApiDoc {
	const blocks = getBlocks(source);

	const aliases: Alias[] = [];
	// Use insertion-order map to preserve declaration order
	const classMap = new Map<string, Class>();
	const nsFuncsMap = new Map<string, Func[]>();

	// First pass: register all class names so method blocks can look them up
	for (const block of blocks) {
		for (const line of block.comments) {
			const m = line.match(/^@class\s+(\w+)/);
			if (m && !classMap.has(m[1])) {
				classMap.set(m[1], { name: m[1], parents: [], desc: [], fields: [], methods: [] });
			}
		}
	}

	// Second pass: full parse
	for (const block of blocks) {
		const hasClass = block.comments.some((l) => l.startsWith('@class'));
		const hasAlias = block.comments.some((l) => l.startsWith('@alias'));

		if (hasClass) {
			const cls = parseClassBlock(block);
			if (cls) {
				const existing = classMap.get(cls.name);
				if (existing) {
					if (cls.parents.length > 0) existing.parents = cls.parents;
					if (cls.desc.length > 0) existing.desc = cls.desc;
					existing.fields.push(...cls.fields);
				}
			}
		} else if (hasAlias) {
			aliases.push(...parseAliasBlock(block));
		} else if (block.codeLine) {
			const func = parseFuncBlock(block as Block & { codeLine: string });
			if (!func) continue;

			if (func.className && classMap.has(func.className)) {
				classMap.get(func.className)!.methods.push(func);
			} else {
				// Bucket by the function's namespace: everything before the last dot, e.g.
				// "lewdware.popup.image" -> "lewdware.popup", "lewdware.exit" -> "lewdware".
				// Handles any `lewdware.x.*` sub-table generically, not just media/monitors.
				const nsKey = func.fullName.slice(0, func.fullName.lastIndexOf('.'));

				if (!nsFuncsMap.has(nsKey)) nsFuncsMap.set(nsKey, []);
				nsFuncsMap.get(nsKey)!.push(func);
			}
		}
	}

	// Insertion order follows first appearance in the source, which puts the flat `lewdware`
	// bucket wherever its first non-namespaced function is declared (not necessarily first) --
	// reorder so it always leads, then the rest keep source order.
	const namespaces = [...nsFuncsMap.entries()]
		.sort(([a], [b]) => (a === 'lewdware' ? -1 : b === 'lewdware' ? 1 : 0))
		.map(([name, functions]) => ({ name, functions }));

	return {
		aliases,
		classes: [...classMap.values()],
		namespaces,
	};
}
