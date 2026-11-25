// Marks a 32-bit little-endian ELF's stack executable (PF_X on PT_GNU_STACK), as `execstack -s` does.
import { readFileSync, writeFileSync } from "node:fs";

const file = process.argv[2];
const elf = readFileSync(file);
if (elf.readUInt32BE(0) !== 0x7f454c46 || elf[4] !== 1 || elf[5] !== 1) throw new Error(`${file}: not a 32-bit little-endian ELF`);
const PT_GNU_STACK = 0x6474e551;
const PF_X = 1;
const phoff = elf.readUInt32LE(0x1c);
const phentsize = elf.readUInt16LE(0x2a);
const phnum = elf.readUInt16LE(0x2c);
for (let i = 0; i < phnum; i++) {
  const header = phoff + i * phentsize;
  if (elf.readUInt32LE(header) !== PT_GNU_STACK) continue;
  const flags = header + 24;
  elf.writeUInt32LE(elf.readUInt32LE(flags) | PF_X, flags);
  writeFileSync(file, elf);
  console.log(`${file}: stack marked executable`);
  process.exit(0);
}
throw new Error(`${file}: no PT_GNU_STACK header`);
