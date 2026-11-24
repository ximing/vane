#!/usr/bin/env bash
# Strip LLVM bitcode from a Rust staticlib so Go 1.26+ cgo will link it.
#
# cmd/go walks every archive member and rejects:
#   ELF: .llvmbc, .llvmcmd, .gnu.lto_*, .gnu.debuglto_*, .llvm.lto*,
#        and SHT_LLVM_LTO / SHT_LLVM_LINKER_OPTIONS / SHT_LLVM_DEPENDENT_LIBRARIES
#   Mach-O: segment __LLVM or __GNU_LTO
# Recent rustc emits these into staticlib even when LTO is off. The bitcode is
# not used by the cgo link. GNU objcopy handles ELF; Mach-O needs llvm-objcopy
# (rustup component llvm-tools, or the rust-objcopy shipped with the toolchain).
#
# Usage: strip-llvm-bitcode.sh archive.a [archive.a ...]
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <archive.a> [archive.a ...]" >&2
  exit 2
fi

find_objcopy() {
  if [[ -n "${LLVM_OBJCOPY:-}" ]]; then
    echo "$LLVM_OBJCOPY"
    return
  fi
  local candidate
  for candidate in llvm-objcopy rust-objcopy objcopy; do
    if command -v "$candidate" >/dev/null 2>&1; then
      command -v "$candidate"
      return
    fi
  done
  local sysroot host
  sysroot="$(rustc --print sysroot 2>/dev/null || true)"
  host="$(rustc -vV 2>/dev/null | awk '/^host:/{print $2}')"
  if [[ -n "$sysroot" && -n "$host" ]]; then
    for candidate in \
      "$sysroot/lib/rustlib/$host/bin/llvm-objcopy" \
      "$sysroot/lib/rustlib/$host/bin/rust-objcopy"
    do
      if [[ -x "$candidate" ]]; then
        echo "$candidate"
        return
      fi
    done
  fi
  echo "strip-llvm-bitcode: need llvm-objcopy (rustup component add llvm-tools) or objcopy" >&2
  exit 1
}

OBJCOPY="$(find_objcopy)"

python3 - "$OBJCOPY" "$@" <<'PY'
import os
import struct
import subprocess
import sys
import tempfile

OBJCOPY = sys.argv[1]
ARCHIVES = sys.argv[2:]

SHT_LLVM_LINKER_OPTIONS = 0x6FFF4C01
SHT_LLVM_DEPENDENT_LIBRARIES = 0x6FFF4C04
SHT_LLVM_LTO = 0x6FFF4C0C
LLVM_SECTION_TYPES = {
    SHT_LLVM_LINKER_OPTIONS,
    SHT_LLVM_DEPENDENT_LIBRARIES,
    SHT_LLVM_LTO,
}
LC_SEGMENT_64 = 0x19
LC_SEGMENT = 0x1
LC_LINKER_OPTION = 0x2D
MACHO_MAGIC_64 = 0xFEEDFACF
MACHO_MAGIC_32 = 0xFEEDFACE
DISALLOWED_SEGMENTS = {"__LLVM", "__GNU_LTO"}


def fail(msg: str) -> None:
    print(f"strip-llvm-bitcode: {msg}", file=sys.stderr)
    raise SystemExit(1)


def disallowed_elf_name(name: str) -> bool:
    return (
        name.startswith(".gnu.lto_")
        or name.startswith(".gnu.debuglto_")
        or name.startswith(".llvm.lto")
        or name in {".llvmbc", ".llvmcmd"}
    )


def cstr(buf: bytes) -> str:
    return buf.split(b"\x00", 1)[0].decode("ascii", "replace")


def iter_members(data: bytes):
    if data.startswith(b"!<thin>\n"):
        fail("thin archives are not supported")
    if not data.startswith(b"!<arch>\n"):
        fail("not an ar archive")
    off = 8
    while off + 60 <= len(data):
        hdr = data[off : off + 60]
        if hdr[58:60] != b"`\n":
            fail(f"malformed archive header at offset {off}")
        try:
            size = int(hdr[48:58].split()[0])
        except (IndexError, ValueError):
            fail(f"malformed archive member size at offset {off}")
        start = off + 60
        end = start + size
        if end > len(data):
            fail("truncated archive member")
        payload = data[start:end]
        name_field = hdr[:16]
        if name_field.startswith(b"#1/"):
            name_len = int(name_field[3:].strip() or b"0")
            if name_len < 0 or name_len > len(payload):
                fail("malformed BSD archive member name")
            name = cstr(payload[:name_len])
            body = payload[name_len:]
        else:
            name = name_field.decode("ascii", "replace").strip()
            body = payload
        yield name, body
        off = end + (end & 1)


def elf_removals(blob: bytes):
    if len(blob) < 64 or not blob.startswith(b"\x7fELF"):
        return None
    ei_class = blob[4]
    ei_data = blob[5]
    if ei_data != 1:
        fail("big-endian ELF members are not supported")
    if ei_class == 2:
        e_shoff = struct.unpack_from("<Q", blob, 40)[0]
        e_shentsize = struct.unpack_from("<H", blob, 58)[0]
        e_shnum = struct.unpack_from("<H", blob, 60)[0]
        e_shstrndx = struct.unpack_from("<H", blob, 62)[0]
        if e_shentsize < 64:
            fail("ELF64 section header is too small")

        def section_meta(index: int):
            base = e_shoff + index * e_shentsize
            name_off = struct.unpack_from("<I", blob, base)[0]
            sh_type = struct.unpack_from("<I", blob, base + 4)[0]
            sh_offset = struct.unpack_from("<Q", blob, base + 24)[0]
            sh_size = struct.unpack_from("<Q", blob, base + 32)[0]
            return name_off, sh_type, sh_offset, sh_size

    elif ei_class == 1:
        if len(blob) < 52:
            fail("truncated ELF32 header")
        e_shoff = struct.unpack_from("<I", blob, 32)[0]
        e_shentsize = struct.unpack_from("<H", blob, 46)[0]
        e_shnum = struct.unpack_from("<H", blob, 48)[0]
        e_shstrndx = struct.unpack_from("<H", blob, 50)[0]
        if e_shentsize < 40:
            fail("ELF32 section header is too small")

        def section_meta(index: int):
            base = e_shoff + index * e_shentsize
            name_off = struct.unpack_from("<I", blob, base)[0]
            sh_type = struct.unpack_from("<I", blob, base + 4)[0]
            sh_offset = struct.unpack_from("<I", blob, base + 16)[0]
            sh_size = struct.unpack_from("<I", blob, base + 20)[0]
            return name_off, sh_type, sh_offset, sh_size

    else:
        fail(f"unknown ELF class {ei_class}")
    if e_shnum == 0 or e_shstrndx >= e_shnum:
        fail("ELF object is missing a section name table")
    _, _, str_off, str_size = section_meta(e_shstrndx)
    strtab = blob[str_off : str_off + str_size]
    names = []
    for index in range(e_shnum):
        name_off, sh_type, _, _ = section_meta(index)
        if name_off >= len(strtab):
            fail("ELF section name is outside the string table")
        name = cstr(strtab[name_off:])
        if name and (disallowed_elf_name(name) or sh_type in LLVM_SECTION_TYPES):
            names.append(name)
    return names


def macho_removals(blob: bytes):
    if len(blob) < 28:
        return None
    magic = struct.unpack_from("<I", blob, 0)[0]
    if magic == MACHO_MAGIC_64:
        header = 32
        lc_segment = LC_SEGMENT_64
        seg_size = 72
        nsects_at = 64
        sect_size = 80
    elif magic == MACHO_MAGIC_32:
        header = 28
        lc_segment = LC_SEGMENT
        seg_size = 56
        nsects_at = 48
        sect_size = 68
    else:
        return None
    ncmds, sizeofcmds = struct.unpack_from("<II", blob, 16)
    off = header
    end = header + sizeofcmds
    if end > len(blob):
        fail("truncated Mach-O load commands")
    removals = []
    # Go rejects the segment load command itself, including an empty one left
    # behind after its sections are removed.
    segments = []
    for _ in range(ncmds):
        if off + 8 > end:
            fail("truncated Mach-O load command")
        cmd, cmdsize = struct.unpack_from("<II", blob, off)
        if cmdsize < 8 or off + cmdsize > len(blob):
            fail("malformed Mach-O load command")
        if cmd == LC_LINKER_OPTION:
            fail("Mach-O member contains LC_LINKER_OPTION, which Go rejects and this script does not rewrite")
        if cmd == lc_segment and cmdsize >= seg_size:
            segname = cstr(blob[off + 8 : off + 24])
            nsects = struct.unpack_from("<I", blob, off + nsects_at)[0]
            if segname in DISALLOWED_SEGMENTS:
                segments.append(segname)
            sec_off = off + seg_size
            for _section in range(nsects):
                if sec_off + 32 > off + cmdsize:
                    fail("truncated Mach-O section")
                sect = cstr(blob[sec_off : sec_off + 16])
                seg = cstr(blob[sec_off + 16 : sec_off + 32])
                if seg in DISALLOWED_SEGMENTS or disallowed_elf_name(sect):
                    removals.append(f"{seg},{sect}")
                sec_off += sect_size
        off += cmdsize
    return removals, segments


def collect(data: bytes):
    elf_names = []
    macho_names = []
    macho_segments = []
    saw_object = False
    for _name, body in iter_members(data):
        if not body:
            continue
        elf = elf_removals(body)
        if elf is not None:
            saw_object = True
            elf_names.extend(elf)
            continue
        macho = macho_removals(body)
        if macho is not None:
            saw_object = True
            sections, segments = macho
            macho_names.extend(sections)
            macho_segments.extend(segments)
    if not saw_object:
        fail("archive contains no ELF or Mach-O members")
    return elf_names, macho_names, macho_segments


def objcopy_is_llvm() -> bool:
    out = subprocess.run(
        [OBJCOPY, "--version"],
        check=True,
        capture_output=True,
        text=True,
    )
    text = out.stdout + out.stderr
    return "LLVM" in text


def strip_one(path: str) -> None:
    with open(path, "rb") as fh:
        data = fh.read()
    elf_names, macho_names, macho_segments = collect(data)
    removals = list(dict.fromkeys([*elf_names, *macho_names]))
    if not removals and not macho_segments:
        print(f"strip-llvm-bitcode: {path} already has no LTO sections")
        return
    if (macho_names or macho_segments) and not objcopy_is_llvm():
        fail(
            f"{path} is a Mach-O archive; GNU objcopy cannot strip __LLVM. "
            "Install llvm-objcopy (rustup component add llvm-tools)."
        )
    if macho_segments and not removals:
        fail(f"{path} still has Mach-O segment(s) {', '.join(macho_segments)} with no sections to remove")
    args = [f"--remove-section={name}" for name in removals]
    fd, tmp = tempfile.mkstemp(prefix=".strip-", dir=os.path.dirname(path) or ".")
    os.close(fd)
    try:
        subprocess.run([OBJCOPY, *args, path, tmp], check=True)
        with open(tmp, "rb") as fh:
            after = fh.read()
        left_elf, left_macho, left_segments = collect(after)
        left = list(dict.fromkeys([*left_elf, *left_macho, *left_segments]))
        if left:
            fail(f"still present in {path}: {', '.join(left)}")
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)
    print(f"strip-llvm-bitcode: removed {', '.join(removals)} from {path}")


for archive in ARCHIVES:
    if not os.path.isfile(archive):
        fail(f"not a file: {archive}")
    strip_one(archive)
PY
