export default { outboundService: () => { try { return fetch(url); } catch { return Response.error(); } } };
