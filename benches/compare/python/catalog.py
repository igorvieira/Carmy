"""Same catalog and search as benches/compare/src/lib.rs."""

PRODUCTS = [
    ("KB-01", "Mechanical keyboard", 12900), ("KB-02", "Low-profile keyboard", 9900),
    ("MS-01", "Wireless mouse", 4900), ("MS-02", "Vertical mouse", 5900),
    ("MS-03", "Trackball mouse", 7900), ("MN-01", "4K monitor", 39900),
    ("MN-02", "Ultrawide monitor", 54900), ("HD-01", "USB-C hub", 3900),
    ("HP-01", "Noise-cancelling headphones", 24900), ("WC-01", "1080p webcam", 6900),
    ("MC-01", "USB microphone", 11900), ("DS-01", "Standing desk", 49900),
    ("CH-01", "Ergonomic chair", 39900), ("LP-01", "Desk lamp", 2900),
    ("PD-01", "Mouse pad", 1900), ("CB-01", "Thunderbolt cable", 2400),
]


def search(query: str) -> dict:
    q = query.lower()
    return {
        "products": [
            {"sku": sku, "name": name, "price_cents": price}
            for sku, name, price in PRODUCTS
            if q in name.lower()
        ]
    }
