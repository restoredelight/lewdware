import type { APIRoute } from 'astro';
import latest from '../../data/config.schema.json';

export const GET: APIRoute = () => {
	return new Response(JSON.stringify(latest, null, 2), {
		headers: { 'Content-Type': 'application/json' },
	});
};
