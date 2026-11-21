const observer = new MutationObserver(callback); observer.observe(element, options); observer.disconnect(); observer.takeRecords();
