local values = {}
local seed = string.rep("a", 65536)
while true do
  values[#values + 1] = seed .. #values
end
