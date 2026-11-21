export default {outboundService: () => new Response("fake"), serviceBindings: {API: () => Response.json({ok: true})}};
