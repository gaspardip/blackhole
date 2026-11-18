function response(request) { if (request) return Response.json({}); return new Response(); } export default { outboundService: (request) => response(request) };
