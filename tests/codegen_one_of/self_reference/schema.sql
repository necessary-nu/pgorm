CREATE TABLE "topic" ( "id" serial NOT NULL, "title" varchar NOT NULL, PRIMARY KEY ("id") );
CREATE TABLE "remark" ( "id" serial NOT NULL, "body" varchar NOT NULL, "topic_id" integer, "reply_to_id" integer, PRIMARY KEY ("id"), CONSTRAINT "fk-remark-reply_to_id" FOREIGN KEY ("reply_to_id") REFERENCES "remark" ("id"), CONSTRAINT "fk-remark-topic_id" FOREIGN KEY ("topic_id") REFERENCES "topic" ("id") );
ALTER TABLE "remark" ADD CONSTRAINT "ck-remark-subject" CHECK (num_nonnulls("topic_id", "reply_to_id") = 1);
