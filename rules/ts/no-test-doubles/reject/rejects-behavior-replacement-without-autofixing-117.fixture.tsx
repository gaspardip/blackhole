const replacement = { fetch: request }; const nested = { ...replacement }; Object.assign(window, { ...nested });
