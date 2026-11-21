const key = selectKey(); const original = router[key]; router[key] = () => false; router[key] = original;
