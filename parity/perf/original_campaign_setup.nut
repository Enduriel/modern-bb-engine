// Performance baselines in the original, campaign mode: runs just before the campaign starts.
::__perfCampaignStart <- ::Time.getExactTime();
::logInfo("PROBE perf mark campaign_start = t=" + ::__perfCampaignStart);
