export default { outboundService: (request) => fetch(request), serviceBindings: { API: (request) => env.ASSETS.fetch(request) } };
