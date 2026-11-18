const original = router.navigate; router.navigate = () => false; router.navigate = original;
