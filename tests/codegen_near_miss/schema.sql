CREATE TYPE "shelf_kind" AS ENUM ('book', 'map');
CREATE TABLE "shelf" ( "id" serial NOT NULL, "kind" shelf_kind NOT NULL, "aisle" integer, "label" varchar, PRIMARY KEY ("id"), CONSTRAINT "uq-shelf-id-kind" UNIQUE ("id", "kind"), UNIQUE NULLS NOT DISTINCT ("aisle", "label") );
CREATE TABLE "book" ( "id" integer NOT NULL, "title" varchar NOT NULL, "kind" shelf_kind GENERATED ALWAYS AS ('book') STORED, PRIMARY KEY ("id"), CONSTRAINT "fk-book-id-kind" FOREIGN KEY ("id", "kind") REFERENCES "shelf" ("id", "kind") ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE );
