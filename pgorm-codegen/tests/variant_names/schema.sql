CREATE TYPE "outlet" AS ENUM ('web_shop', 'pop_up_shop', 'closed_shop');
CREATE TABLE "web_shop" ( "id" serial NOT NULL, "url" varchar NOT NULL, PRIMARY KEY ("id") );
CREATE TABLE "pop_up_shop" ( "id" serial NOT NULL, "site" varchar NOT NULL, PRIMARY KEY ("id") );
CREATE TABLE "closed_shop" ( "id" serial NOT NULL, PRIMARY KEY ("id") );
CREATE TABLE "sale" ( "id" serial NOT NULL, "outlet" outlet NOT NULL, "web_shop_id" integer, "pop_up_shop_id" integer, "closed_shop_id" integer, PRIMARY KEY ("id"), FOREIGN KEY ("web_shop_id") REFERENCES "web_shop" ("id"), FOREIGN KEY ("pop_up_shop_id") REFERENCES "pop_up_shop" ("id"), FOREIGN KEY ("closed_shop_id") REFERENCES "closed_shop" ("id") );
