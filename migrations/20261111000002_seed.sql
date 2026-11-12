-- Starting catalog and prices. Everything here can be changed later in the
-- admin page; prices are examples, set your own before launch.

INSERT INTO garments (slug, kind, name_uz, name_ru, name_en, base_price_tiyin, sort) VALUES
    ('tshirt',     'tshirt',     'Futbolka', 'Футболка', 'T-shirt',    9000000, 1),
    ('hoodie',     'hoodie',     'Xudi',     'Худи',     'Hoodie',    22000000, 2),
    ('sweatshirt', 'sweatshirt', 'Svitshot', 'Свитшот',  'Sweatshirt', 18000000, 3);

INSERT INTO garment_colors (garment_id, slug, name_uz, name_ru, name_en, hex, sort)
SELECT g.id, c.slug, c.name_uz, c.name_ru, c.name_en, c.hex, c.sort
FROM garments g
CROSS JOIN (VALUES
    ('black', 'Qora',       'Чёрный',  'Black', '#18181b', 1),
    ('white', 'Oq',         'Белый',   'White', '#f5f5f4', 2),
    ('grey',  'Kulrang',    'Серый',   'Grey',  '#9ca3af', 3),
    ('navy',  'Toʻq koʻk',  'Тёмно-синий', 'Navy', '#1e293b', 4),
    ('sand',  'Qum rang',   'Песочный', 'Sand', '#d6c7a8', 5)
) AS c (slug, name_uz, name_ru, name_en, hex, sort);

INSERT INTO garment_sizes (garment_id, size, sort)
SELECT g.id, s.size, s.sort
FROM garments g
CROSS JOIN (VALUES ('S', 1), ('M', 2), ('L', 3), ('XL', 4), ('2XL', 5)) AS s (size, sort);

-- mock_* place the area on the 1000x1100 garment artwork. The ratios keep
-- centimetres square on screen: mock_w * 1000 / width_cm = mock_h * 1100 / height_cm.
INSERT INTO print_areas (garment_id, side, width_cm, height_cm, mock_x, mock_y, mock_w, mock_h)
SELECT g.id, a.side, a.width_cm, a.height_cm, a.mock_x, a.mock_y, a.mock_w, a.mock_h
FROM garments g
JOIN (VALUES
    ('tshirt',     'front', 30, 40, 0.3665, 0.2273, 0.267, 0.3236),
    ('tshirt',     'back',  30, 40, 0.3665, 0.2000, 0.267, 0.3236),
    ('hoodie',     'front', 30, 35, 0.3810, 0.2909, 0.238, 0.2524),
    ('hoodie',     'back',  30, 40, 0.3810, 0.3273, 0.238, 0.2885),
    ('sweatshirt', 'front', 30, 40, 0.3770, 0.2364, 0.246, 0.2982),
    ('sweatshirt', 'back',  30, 40, 0.3770, 0.2000, 0.246, 0.2982)
) AS a (garment, side, width_cm, height_cm, mock_x, mock_y, mock_w, mock_h) ON a.garment = g.slug;

INSERT INTO price_tiers (max_area_cm2, price_tiyin) VALUES
    (100,  2000000),
    (300,  3500000),
    (600,  5000000),
    (1200, 7000000);

INSERT INTO quantity_discounts (min_qty, percent_bp) VALUES
    (5,  500),
    (10, 1000),
    (30, 1500);

INSERT INTO regions (slug, name_uz, name_ru, name_en, delivery_fee_tiyin, sort) VALUES
    ('tashkent-city',  'Toshkent shahri',           'город Ташкент',            'Tashkent city',        2500000, 1),
    ('tashkent',       'Toshkent viloyati',         'Ташкентская область',      'Tashkent region',      3500000, 2),
    ('andijan',        'Andijon viloyati',          'Андижанская область',      'Andijan region',       4500000, 3),
    ('bukhara',        'Buxoro viloyati',           'Бухарская область',        'Bukhara region',       4500000, 4),
    ('fergana',        'Fargʻona viloyati',         'Ферганская область',       'Fergana region',       4500000, 5),
    ('jizzakh',        'Jizzax viloyati',           'Джизакская область',       'Jizzakh region',       4000000, 6),
    ('namangan',       'Namangan viloyati',         'Наманганская область',     'Namangan region',      4500000, 7),
    ('navoiy',         'Navoiy viloyati',           'Навоийская область',       'Navoiy region',        4500000, 8),
    ('kashkadarya',    'Qashqadaryo viloyati',      'Кашкадарьинская область',  'Kashkadarya region',   4500000, 9),
    ('samarkand',      'Samarqand viloyati',        'Самаркандская область',    'Samarkand region',     4000000, 10),
    ('sirdarya',       'Sirdaryo viloyati',         'Сырдарьинская область',    'Sirdarya region',      4000000, 11),
    ('surkhandarya',   'Surxondaryo viloyati',      'Сурхандарьинская область', 'Surkhandarya region',  5000000, 12),
    ('khorezm',        'Xorazm viloyati',           'Хорезмская область',       'Khorezm region',       5000000, 13),
    ('karakalpakstan', 'Qoraqalpogʻiston Respublikasi', 'Республика Каракалпакстан', 'Republic of Karakalpakstan', 5500000, 14);

INSERT INTO settings (key, value) VALUES
    ('pickup_address', '{"uz": "Toshkent shahri (aniq manzil buyurtma tasdiqlanganda aytiladi)", "ru": "г. Ташкент (точный адрес сообщим при подтверждении заказа)", "en": "Tashkent city (the exact address is given when the order is confirmed)"}');
