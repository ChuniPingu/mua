def read_and_split(file_path):
    with open(file_path, 'r') as file:
        lines = [line.strip().split() for line in file if line.strip()]

    max_cols = max(len(line) for line in lines)

    result = []
    for col in range(max_cols):
        for row in range(len(lines)):
            if col < len(lines[row]):
                result.append(lines[row][col])
    return result


if __name__ == "__main__":
    import sys

    if len(sys.argv) != 2:
        print("Usage: python split.py <file_path>")
        sys.exit(1)

    file_path = sys.argv[1]
    words = read_and_split(file_path)
    print(" ".join(words))
