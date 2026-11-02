let bit_index = 1u128 << (quest_id as usize);
   require!((profile.quest_bitmask & bit_index) == 0, ErrorCode::QuestAlreadyCompleted);
   profile.quest_bitmask |= bit_index;