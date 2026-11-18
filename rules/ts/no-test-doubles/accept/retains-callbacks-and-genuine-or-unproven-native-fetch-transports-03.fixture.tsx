type Factory = () => Promise<Response>; const factory: Factory = async () => new Response();
