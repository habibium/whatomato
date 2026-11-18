-- The numbers an agent's send tags, each written as @number in its body
ALTER TABLE deliveries ADD COLUMN mentions TEXT[] NOT NULL DEFAULT '{}';
