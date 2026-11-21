export default { outboundService: () => { switch (mode) { case "ok": return new Response(); default: return fetch(url); } } };
