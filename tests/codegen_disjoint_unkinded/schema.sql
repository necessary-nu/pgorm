CREATE TABLE "shop" ( "id" serial NOT NULL, "name" varchar NOT NULL, PRIMARY KEY ("id") );
CREATE TABLE "website" ( "shop_id" integer NOT NULL, "url" varchar NOT NULL, PRIMARY KEY ("shop_id"), FOREIGN KEY ("shop_id") REFERENCES "shop" ("id") );
CREATE TABLE "storefront" ( "id" integer NOT NULL, "street" varchar NOT NULL, PRIMARY KEY ("id"), FOREIGN KEY ("id") REFERENCES "shop" ("id") ON DELETE CASCADE );
CREATE SCHEMA "crm";
CREATE TABLE "crm"."account" ( "tenant" integer NOT NULL, "id" bigint NOT NULL, "name" varchar NOT NULL, PRIMARY KEY ("tenant", "id") );
CREATE TABLE "crm"."person" ( "tenant_id" integer NOT NULL, "account_id" bigint NOT NULL, "given_name" varchar NOT NULL, PRIMARY KEY ("tenant_id", "account_id"), FOREIGN KEY ("tenant_id", "account_id") REFERENCES "crm"."account" ("tenant", "id") );
CREATE TABLE "crm"."company" ( "tenant_id" integer NOT NULL, "account_id" bigint NOT NULL, "vat" varchar NOT NULL, PRIMARY KEY ("tenant_id", "account_id"), FOREIGN KEY ("account_id", "tenant_id") REFERENCES "crm"."account" ("id", "tenant") );
