import type { Pool } from "@necessary/pgorm";

/**
 * What the guide's test hands an example that names `pool` or `url`: a pool
 * over a scratch database of the example's own, holding the bakery unless
 * the chapter marks the example `<!-- example: empty -->`, and that
 * database's connection string.
 */
export interface GuideContext {
  readonly pool: Pool;
  readonly url: string;
}
