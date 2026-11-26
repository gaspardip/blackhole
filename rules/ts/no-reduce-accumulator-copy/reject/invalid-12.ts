declare const items: number[];
items.reduceRight((acc, item, index) => acc['concat']([item]), [] as Item[]);
