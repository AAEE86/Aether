ALTER TABLE public.routing_groups
    ADD COLUMN sort_order bigint NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS routing_groups_enabled_sort_idx
    ON public.routing_groups (enabled DESC, sort_order, name, id);
