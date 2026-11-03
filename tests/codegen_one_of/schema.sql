CREATE TABLE "post" ( "id" serial NOT NULL, "title" varchar NOT NULL, PRIMARY KEY ("id") );
CREATE TABLE "photo" ( "id" bigserial NOT NULL, "url" varchar NOT NULL, PRIMARY KEY ("id") );
CREATE TABLE "comment" ( "id" serial NOT NULL, "body" varchar NOT NULL, "post_id" integer, "photo_id" bigint, PRIMARY KEY ("id"), CONSTRAINT "fk-comment-photo_id" FOREIGN KEY ("photo_id") REFERENCES "photo" ("id"), CONSTRAINT "fk-comment-post_id" FOREIGN KEY ("post_id") REFERENCES "post" ("id") );
ALTER TABLE "comment" ADD CONSTRAINT "ck-comment-commentable" CHECK (num_nonnulls("post_id", "photo_id") <= 1);
