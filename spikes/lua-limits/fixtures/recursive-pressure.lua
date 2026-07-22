local function descend()
  return 1 + descend()
end
return descend()
