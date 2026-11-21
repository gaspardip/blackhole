function response() { return new Response(); } function upstream(name) { return async (request) => response(request, name); } export default { serviceBindings: { API: upstream("api") } };
