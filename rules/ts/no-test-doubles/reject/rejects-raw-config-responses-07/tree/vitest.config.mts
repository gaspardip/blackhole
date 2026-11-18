const responder = async () => { const value = Response.json({}); return await value; }; const bindings = { API: responder }; export default { serviceBindings: { ...bindings } };
