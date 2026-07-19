(_input) => {
  function recurse() {
    return recurse() + 1
  }
  return recurse()
}
