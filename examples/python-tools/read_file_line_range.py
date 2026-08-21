from pathlib import Path


def read_file_line_range(
    file_name: str,
    start_line: int,
    end_line: int,
) -> str:
    """Read only the requested inclusive line range from a UTF-8 text file.

    Prefer this tool for partial file inspection instead of reading an entire
    file, especially when the file may be large.
    """
    if start_line < 1:
        raise ValueError("start_line must be at least 1")
    if end_line < start_line:
        raise ValueError("end_line must be greater than or equal to start_line")

    max_lines = 500
    if end_line - start_line + 1 > max_lines:
        raise ValueError(f"line range cannot exceed {max_lines} lines")

    path = Path(file_name).expanduser()
    selected = []

    with path.open("r", encoding="utf-8") as file:
        for line_number, line in enumerate(file, start=1):
            if line_number > end_line:
                break
            if line_number >= start_line:
                selected.append(line)

    return "".join(selected)
