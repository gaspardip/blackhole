const original = router.navigate; other.navigate = () => false; router.navigate = original;
