CREATE TYPE "fleet_kind" AS ENUM ('car', 'bike');
CREATE TABLE "fleet" ( "id" serial NOT NULL, "kind" fleet_kind NOT NULL, "name" varchar NOT NULL, PRIMARY KEY ("id"), CONSTRAINT "uq-fleet-id-kind" UNIQUE ("id", "kind") );
CREATE TABLE "car" ( "id" integer NOT NULL, "seats" smallint NOT NULL, "kind" fleet_kind GENERATED ALWAYS AS ('car') STORED, PRIMARY KEY ("id"), CONSTRAINT "fk-car-id-kind" FOREIGN KEY ("id", "kind") REFERENCES "fleet" ("id", "kind") ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE );
ALTER TABLE "fleet" ADD COLUMN "car_id" integer GENERATED ALWAYS AS ((CASE WHEN ("kind" = 'car') THEN "id" END)) STORED, ADD CONSTRAINT "fk-fleet-car_id" FOREIGN KEY ("car_id") REFERENCES "car" ("id") DEFERRABLE INITIALLY IMMEDIATE;
